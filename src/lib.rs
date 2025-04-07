#![no_std]
#![feature(allocator_api, sync_unsafe_cell, ptr_as_ref_unchecked)]

use core::{
    cell::{SyncUnsafeCell, UnsafeCell},
    future::Future,
    ops::AddAssign,
    pin::Pin,
    task::Poll,
};

use alloc::{boxed::Box, collections::btree_map::BTreeMap, vec::Vec};
use async_lock::RwLock;

extern crate alloc;
pub trait NoEndFuture: Future<Output = ()> + Send + Sync {}

pub struct DynamicJoinArray {
    job_queue: RwLock<BTreeMap<usize, Pin<Box<dyn NoEndFuture>>>>,
    cycle_counter: RwLock<usize>,
}

impl DynamicJoinArray {
    pub fn new() -> Self {
        Self {
            job_queue: BTreeMap::new().into(),
            cycle_counter: 0.into(),
        }
    }

    pub async fn add(&self, fut: Pin<Box<dyn NoEndFuture>>) -> usize {
        let mut guard = self.cycle_counter.write().await;
        let id = *guard;
        guard.add_assign(1);
        self.job_queue.write().await.insert(id, fut);
        id
    }

    pub async fn drop(&self, id: &usize) -> Option<Pin<Box<dyn NoEndFuture>>> {
        self.job_queue.write().await.remove(id)
    }

    pub fn work<'a>(&'a self) -> DynamicJoinArrayFuture<'a> {
        DynamicJoinArrayFuture { ref_to_array: self }
    }
}

pub struct DynamicJoinArrayFuture<'a> {
    ref_to_array: &'a DynamicJoinArray,
}

impl<'a> Future for DynamicJoinArrayFuture<'a> {
    type Output = ();

    fn poll<'b>(
        self: core::pin::Pin<&'b mut Self>,
        cx: &mut core::task::Context<'_>,
    ) -> core::task::Poll<Self::Output> {
        let this = unsafe { self.get_unchecked_mut() };
        match this.ref_to_array.job_queue.try_write() {
            Some(mut queue_ref) => {
                let to_drop = queue_ref
                    .iter_mut()
                    .filter_map(
                        |(id, job)| match unsafe { Pin::new_unchecked(job).poll(cx) } {
                            core::task::Poll::Ready(_) => Some(id.clone()),
                            core::task::Poll::Pending => None,
                        },
                    )
                    .collect::<Vec<_>>();
                if !to_drop.is_empty() {
                    to_drop.iter().for_each(|to_drop_e| {
                        queue_ref.remove(to_drop_e);
                    });
                }
            }
            None => {}
        }
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

    #[tokio::test]
    async fn test_dynamic_join_array() {
        // Create a new DynamicJoinArray
        let mut dynamic_join_array = DynamicJoinArray::new();

        // Add a dummy future
        let future_id = dynamic_join_array.add(Box::pin(DummyFuture)).await;

        // Check that the future has been added to the job queue
        assert!(dynamic_join_array
            .job_queue
            .get_mut()
            .contains_key(&future_id));

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
        assert!(!dynamic_join_array
            .job_queue
            .get_mut()
            .contains_key(&future_id));

        // Drop the future explicitly
        dynamic_join_array.drop(&future_id);

        // Ensure that the future is no longer in the job queue
        assert!(!dynamic_join_array
            .job_queue
            .get_mut()
            .contains_key(&future_id));
    }
}
