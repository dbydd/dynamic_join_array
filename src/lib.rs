#![no_std]
#![feature(allocator_api)]

use core::{future::Future, pin::Pin, task::Poll};

use alloc::{boxed::Box, collections::btree_map::BTreeMap, vec::Vec};
use futures::FutureExt;

extern crate alloc;
pub trait NoEndFuture: Future<Output = ()> + Unpin {}

pub struct DynamicJoinMap {
    job_queue: BTreeMap<usize, Box<dyn Future<Output = ()>>>,
    cycle_counter: usize,
}

impl DynamicJoinMap {
    pub fn add(&mut self, fut: Box<dyn NoEndFuture>) -> usize {
        self.job_queue.insert(self.cycle_counter, fut);
        let idx = self.cycle_counter;
        self.cycle_counter += 1;
        idx
    }
}

pub struct DynamicJoinArray {
    job_queue: BTreeMap<usize, Box<dyn NoEndFuture>>,
    cycle_counter: usize,
}

impl DynamicJoinArray {
    pub fn add(&mut self, fut: Box<dyn NoEndFuture>) -> usize {
        self.job_queue.insert(self.cycle_counter, fut);
        let id = self.cycle_counter;
        self.cycle_counter += 1;
        id
    }

    pub fn drop(&mut self, id: &usize) -> Option<Box<dyn NoEndFuture>> {
        self.job_queue.remove(id)
    }

    pub fn work<'a>(&'a mut self) -> DynamicJoinArrayFuture<'a> {
        DynamicJoinArrayFuture { ref_to_array: self }
    }
}

pub struct DynamicJoinArrayFuture<'a> {
    ref_to_array: &'a mut DynamicJoinArray,
}

impl<'a> Future for DynamicJoinArrayFuture<'a> {
    type Output = ();

    fn poll<'b>(
        self: core::pin::Pin<&'b mut Self>,
        cx: &mut core::task::Context<'_>,
    ) -> core::task::Poll<Self::Output> {
        let this = unsafe { self.get_unchecked_mut() };
        this.ref_to_array
            .job_queue
            .iter_mut()
            .filter_map(
                |(id, job)| match unsafe { Pin::new_unchecked(job).poll(cx) } {
                    core::task::Poll::Ready(_) => Some(id.clone()),
                    core::task::Poll::Pending => None,
                },
            )
            .collect::<Vec<_>>()
            .iter()
            .for_each(|to_drop| {
                this.ref_to_array.job_queue.remove(to_drop);
            });

        Poll::Pending
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use core::future::{Future, IntoFuture};
    use futures::task::Poll;

    // Dummy future that completes immediately
    struct DummyFuture;

    impl Future for DummyFuture {
        type Output = ();

        fn poll(self: Pin<&mut Self>, _: &mut core::task::Context<'_>) -> Poll<Self::Output> {
            Poll::Ready(())
        }
    }
    impl Unpin for DummyFuture {}
    impl NoEndFuture for DummyFuture {}

    #[test]
    fn test_dynamic_join_array() {
        // Create a new DynamicJoinArray
        let mut dynamic_join_array = DynamicJoinArray {
            job_queue: BTreeMap::new(),
            cycle_counter: 0,
        };

        // Add a dummy future
        let future_id = dynamic_join_array.add(Box::new(DummyFuture));

        // Check that the future has been added to the job queue
        assert!(dynamic_join_array.job_queue.contains_key(&future_id));

        // Simulate polling the future (this would normally happen during runtime)
        let mut context = core::task::Context::from_waker(futures::task::noop_waker_ref());
        let dynamic_join_array_future = &mut dynamic_join_array.work().into_future();
        let mut pinned = unsafe { Pin::new_unchecked(dynamic_join_array_future) };

        for _ in (0..1000) {
            match pinned.as_mut().poll(&mut context) {
                Poll::Pending => {} // Future completed
                Poll::Ready(()) => panic!("Future should not be completed"),
            }
        }

        // After polling, the future should be removed from the job queue
        assert!(!dynamic_join_array.job_queue.contains_key(&future_id));

        // Drop the future explicitly
        dynamic_join_array.drop(&future_id);

        // Ensure that the future is no longer in the job queue
        assert!(!dynamic_join_array.job_queue.contains_key(&future_id));
    }
}
