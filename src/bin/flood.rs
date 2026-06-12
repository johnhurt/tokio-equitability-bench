//! Global-queue-mutex-bound workload.
//!
//! `producers` external OS threads (not runtime workers) hammer
//! [`Handle::spawn`]. Every spawn from a non-worker thread locks the runtime's
//! global `synced` mutex to push onto the injection queue; the worker threads
//! lock the same mutex to pull batches back off it. The batch size is set by
//! the proposed `Builder::global_queue_share_per_worker(f)` knob: a worker takes
//! `ceil(len * f)` of the `len` queued tasks per lock. `f = 1.0` lets a worker
//! drain the whole queue in one lock (fewest acquisitions); a small `f` takes a
//! sliver per lock (most contention). A `share` argument of `0` (or less) here
//! leaves the knob unset, so the runtime keeps its default `1 / N` share.
//!
//! `spin` is the per-task CPU cost (a proxy for how much real work each task
//! does). `spin = 0` makes the workload purely scheduling-bound, so the global
//! mutex dominates; a large `spin` makes it work-bound, where the scheduler is
//! a small fraction and the knob no longer matters.
//!
//! Completion is counted via a process-global atomic so there is no per-task
//! `Arc` clone to skew the measurement.
//!
//! Usage:
//!     flood <workers> <share> <producers> <total_ops> <spin>
//! Defaults: 96 1.0 24 5000000 0  (share <= 0 => default 1/N behaviour)

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

fn main() {
    let workers: usize = arg(1, 96);
    let share: f32 = arg(2, 1.0); // <= 0 leaves the knob unset (default 1/N)
    let producers: usize = arg(3, 24);
    let total_ops: u64 = arg(4, 5_000_000);
    let spin: u64 = arg(5, 0);

    let mut builder = tokio::runtime::Builder::new_multi_thread();
    builder.worker_threads(workers);
    if share > 0.0 {
        builder.global_queue_share_per_worker(share);
    }
    let rt = builder.build().unwrap();
    let handle = rt.handle().clone();

    let per = total_ops / producers as u64;
    let real_ops = per * producers as u64;

    // Sample the global (injection) queue depth from a side thread.
    let stop = Arc::new(AtomicBool::new(false));
    let max_depth = Arc::new(AtomicUsize::new(0));
    let depth_sum = Arc::new(AtomicU64::new(0));
    let depth_n = Arc::new(AtomicU64::new(0));
    let sampler = {
        let h = handle.clone();
        let (stop, max_depth, depth_sum, depth_n) =
            (stop.clone(), max_depth.clone(), depth_sum.clone(), depth_n.clone());
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
    let mut prod = Vec::with_capacity(producers);
    for _ in 0..producers {
        let h = handle.clone();
        prod.push(std::thread::spawn(move || {
            for _ in 0..per {
                h.spawn(async move {
                    if spin > 0 {
                        let mut x = 0u64;
                        for i in 0..spin {
                            x = x.wrapping_mul(6364136223846793005).wrapping_add(i);
                        }
                        black_box(x);
                    }
                    DONE.fetch_add(1, Relaxed);
                });
            }
        }));
    }
    for p in prod {
        p.join().unwrap();
    }
    while DONE.load(Relaxed) < real_ops {
        std::thread::sleep(Duration::from_micros(200));
    }
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

    println!(
        "{{\"workers\":{workers},\"share\":{share},\"producers\":{producers},\"spin\":{spin},\"ops\":{real_ops},\"elapsed_s\":{secs:.4},\"throughput_ops_per_s\":{:.0},\"overflows\":{overflows},\"remote_schedules\":{remote},\"steals\":{steals},\"gq_depth_max\":{},\"gq_depth_avg\":{avg_depth:.1}}}",
        real_ops as f64 / secs,
        max_depth.load(Relaxed)
    );
}
