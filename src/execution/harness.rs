use client::{LobstersClient, LobstersRequest};
use crossbeam_channel;
use execution::{self, id_to_slug, LobstersSampler, Sampler};
use rand::{self, Rng};
use std::sync::{Arc, Barrier, Mutex};
use std::{thread, time};
use tokio_core;
use WorkerCommand;
use BASE_OPS_PER_MIN;

pub(crate) fn run<C, I>(
    load: execution::Workload,
    in_flight: usize,
    mut factory: I,
    prime: bool,
) -> (
    f64,
    Vec<thread::JoinHandle<(f64, execution::Stats, execution::Stats)>>,
    usize,
)
where
    I: Send + 'static,
    C: LobstersClient<Factory = I> + 'static,
{
    // generating a request takes a while because we have to generate random numbers (including
    // zipfs). so, depending on the target load, we may need more than one load generation
    // thread. we'll make them all share the pool of issuers though.
    let mut target = BASE_OPS_PER_MIN as f64 * load.req_scale / 60.0;
    let generator_capacity = 100_000.0; // req/s == 10 µs to generate a request
    let ngen = (target / generator_capacity).ceil() as usize;
    target /= ngen as f64;

    let nthreads = load.threads;
    let warmup = load.warmup;
    let runtime = load.runtime;

    if prime {
        C::setup(&mut factory);
    }

    let factory = Arc::new(Mutex::new(factory));
    let (pool, jobs) = crossbeam_channel::unbounded();
    let workers: Vec<_> = (0..nthreads)
        .map(|i| {
            let jobs = jobs.clone();
            let factory = factory.clone();
            thread::Builder::new()
                .name(format!("issuer-{}", i))
                .spawn(move || {
                    let core = tokio_core::reactor::Core::new().unwrap();
                    let c = C::spawn(&mut *factory.lock().unwrap(), &core.handle());

                    execution::issuer::run(warmup, runtime, in_flight, core, c, jobs)
                    // NOTE: there may still be a bunch of requests in the queue here,
                    // but core.run() will return when the stream is closed.
                })
                .unwrap()
        })
        .collect();

    let barrier = Arc::new(Barrier::new(nthreads + 1));
    let now = time::Instant::now();

    // compute how many of each thing there will be in the database after scaling by mem_scale
    let sampler = LobstersSampler::new(load.mem_scale);
    let nstories = sampler.nstories();

    // then, log in all the users
    for u in 0..sampler.nusers() {
        pool.send(WorkerCommand::Request(now, Some(u), LobstersRequest::Login))
            .unwrap();
    }

    if prime {
        println!("--> priming database");
        let mut rng = rand::thread_rng();

        // wait for all threads to be ready
        // we don't normally know which worker thread will receive any given command (it's mpmc
        // after all), but since a ::Wait causes the receiving thread to *block*, we know that once
        // it receives one, it can't receive another until the barrier has been passed! Therefore,
        // sending `nthreads` barriers should ensure that every thread gets one
        for _ in 0..nthreads {
            pool.send(WorkerCommand::Wait(barrier.clone())).unwrap();
        }
        barrier.wait();

        // first, we need to prime the database stories!
        for id in 0..nstories {
            // NOTE: we're assuming that users who vote much also submit many stories
            let req = LobstersRequest::Submit {
                id: id_to_slug(id),
                title: format!("Base article {}", id),
            };
            pool.send(WorkerCommand::Request(
                now,
                Some(sampler.user(&mut rng)),
                req,
            )).unwrap();
        }

        // and as many comments
        for id in 0..sampler.ncomments() {
            let story = id % nstories; // TODO: distribution

            // synchronize occasionally to ensure that we can safely generate parent comments
            if story == 0 {
                for _ in 0..nthreads {
                    pool.send(WorkerCommand::Wait(barrier.clone())).unwrap();
                }
                barrier.wait();
            }

            let parent = if rng.gen_weighted_bool(2) {
                // we need to pick a parent in the same story
                let generated_comments = id - story;
                // how many stories to we know there are per story?
                let generated_comments_per_story = generated_comments / nstories;
                // pick the nth comment to chosen story
                if generated_comments_per_story != 0 {
                    let story_comment = rng.gen_range(0, generated_comments_per_story);
                    Some(story + nstories * story_comment)
                } else {
                    None
                }
            } else {
                None
            };

            let req = LobstersRequest::Comment {
                id: id_to_slug(id),
                story: id_to_slug(story),
                parent: parent.map(id_to_slug),
            };

            // NOTE: we're assuming that users who vote much also submit many stories
            pool.send(WorkerCommand::Request(
                now,
                Some(sampler.user(&mut rng)),
                req,
            )).unwrap();
        }

        // wait for all threads to finish priming comments
        // the addition of the ::Wait barrier will also ensure that start is (re)set
        for _ in 0..nthreads {
            pool.send(WorkerCommand::Wait(barrier.clone())).unwrap();
        }
        barrier.wait();
        println!("--> finished priming database");
    }

    // wait for all threads to be ready (and set their start time correctly)
    for _ in 0..nthreads {
        pool.send(WorkerCommand::Start(barrier.clone())).unwrap();
    }
    barrier.wait();

    let generators: Vec<_> = (0..ngen)
        .map(|geni| {
            let pool = pool.clone();
            let load = load.clone();
            let sampler = sampler.clone();

            thread::Builder::new()
                .name(format!("load-gen{}", geni))
                .spawn(move || {
                    execution::generator::run::<C, LobstersSampler>(load, sampler, pool, target)
                })
                .unwrap()
        })
        .collect();

    drop(pool);
    let gps = generators.into_iter().map(|gen| gen.join().unwrap()).sum();

    // how many operations were left in the queue at the end?
    let dropped = jobs.iter().count();

    (gps, workers, dropped)
}
