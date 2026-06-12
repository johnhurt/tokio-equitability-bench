//! File-server-shaped workload (uses tokio's blocking pool).
//!
//! `req_concurrency` in-flight request tasks; each request fans out
//! `reads_per_req` `spawn_blocking` "file reads", awaits them, then completes.
//! Every read completion wakes the request from a blocking-pool thread, which
//! pushes onto the global task queue (the contended `synced` mutex). Workers
//! pull batches back off it, sized by `global_queue_share_per_worker`.
//!
//! `read_spin` is the per-read CPU cost, a proxy for file size / cache
//! temperature.
//!
//! This binary illustrates an important limitation: a file server built on the
//! default blocking pool tends to bottleneck on the *blocking pool* itself
//! before the runtime's global-queue mutex becomes the limiter, so the
//! share knob has little effect on throughput here. To reach the
//! global-queue-mutex-bound regime, completions must reach the runtime faster
//! than the blocking pool allows (e.g. an io_uring / custom completion backend
//! feeding many threads) — see `flood`.
//!
//! Usage:
//!     file_server <workers> <share> <req_concurrency> <reads_per_req> <read_spin> <total_requests> <blocking_threads>
//! Defaults: 128 1.0 4096 4 0 1000000 512  (share <= 0 => default 1/N behaviour)

use std::env;
use std::hint::black_box;
use std::sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering::Relaxed};
use std::sync::Arc;
use std::time::{Duration, Instant};

static DONE: AtomicU64 = AtomicU64::new(0);

fn arg<T: std::str::FromStr>(i: usize, default: T) -> T {
    env::args()
        .nth(i)
        .and_then(|s| s.parse().ok())
        .unwrap_or(default)
}

#[inline(never)]
fn read_work(spin: u64) -> u64 {
    let mut x = 0u64;
    for i in 0..spin {
        x = x.wrapping_mul(6364136223846793005).wrapping_add(i);
    }
    x
}

fn main() {
    let workers: usize = arg(1, 128);
    let share: f32 = arg(2, 1.0); // <= 0 leaves the knob unset (default 1/N)
    let req_concurrency: usize = arg(3, 4096);
    let reads_per_req: usize = arg(4, 4);
    let read_spin: u64 = arg(5, 0);
    let total_requests: u64 = arg(6, 1_000_000);
    let blocking_threads: usize = arg(7, 512);

    let mut builder = tokio::runtime::Builder::new_multi_thread();
    builder
        .worker_threads(workers)
        .max_blocking_threads(blocking_threads);
    if share > 0.0 {
        builder.global_queue_share_per_worker(share);
    }
    let rt = builder.build().unwrap();

    let per = (total_requests / req_concurrency as u64).max(1);
    let real_reqs = per * req_concurrency as u64;

    let stop = Arc::new(AtomicBool::new(false));
    let max_depth = Arc::new(AtomicUsize::new(0));
    let depth_sum = Arc::new(AtomicU64::new(0));
    let depth_n = Arc::new(AtomicU64::new(0));
    let sampler = {
        let h = rt.handle().clone();
        let (stop, max_depth, depth_sum, depth_n) = (
            stop.clone(),
            max_depth.clone(),
            depth_sum.clone(),
            depth_n.clone(),
        );
        std::thread::spawn(move || {
            let m = h.metrics();
            while !stop.load(Relaxed) {
                let d = m.global_queue_depth();
                max_depth.fetch_max(d, Relaxed);
                depth_sum.fetch_add(d as u64, Relaxed);
                depth_n.fetch_add(1, Relaxed);
                std::thread::sleep(Duration::from_micros(500));
            }
        })
    };

    let start = Instant::now();
    rt.block_on(async move {
        let mut reqs = Vec::with_capacity(req_concurrency);
        for _ in 0..req_concurrency {
            reqs.push(tokio::spawn(async move {
                for _ in 0..per {
                    let mut reads = Vec::with_capacity(reads_per_req);
                    for _ in 0..reads_per_req {
                        reads.push(tokio::task::spawn_blocking(move || {
                            black_box(read_work(read_spin))
                        }));
                    }
                    for h in reads {
                        let _ = h.await.unwrap();
                    }
                    DONE.fetch_add(1, Relaxed);
                }
            }));
        }
        for h in reqs {
            h.await.unwrap();
        }
    });
    let secs = start.elapsed().as_secs_f64();

    stop.store(true, Relaxed);
    let _ = sampler.join();

    let m = rt.metrics();
    let nw = m.num_workers();
    let mut overflows = 0u64;
    let mut steals = 0u64;
    for i in 0..nw {
        overflows += m.worker_overflow_count(i);
        steals += m.worker_steal_count(i);
    }
    let remote = m.remote_schedule_count();
    let dn = depth_n.load(Relaxed).max(1);
    let avg_depth = depth_sum.load(Relaxed) as f64 / dn as f64;
    let reads = real_reqs * reads_per_req as u64;

    println!(
        "{{\"workers\":{workers},\"share\":{share},\"req_conc\":{req_concurrency},\"reads_per_req\":{reads_per_req},\"read_spin\":{read_spin},\"requests\":{real_reqs},\"reads\":{reads},\"blocking_threads\":{blocking_threads},\"elapsed_s\":{secs:.4},\"req_per_s\":{:.0},\"reads_per_s\":{:.0},\"overflows\":{overflows},\"remote_schedules\":{remote},\"steals\":{steals},\"gq_depth_avg\":{avg_depth:.1}}}",
        real_reqs as f64 / secs,
        reads as f64 / secs
    );
}
