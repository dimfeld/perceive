use std::{fmt::Debug, ops::Deref, sync::Arc};

use crossbeam::queue::SegQueue;
use parking_lot::Mutex;

use crate::sources::pipeline::CountingVecSender;

#[derive(Debug)]
pub struct BatchSender<'a, T: Send + Sync>(Arc<BatchSenderInner<'a, T>>);

impl<'a, T: Send + Sync> Clone for BatchSender<'a, T> {
    fn clone(&self) -> Self {
        Self(self.0.clone())
    }
}

impl<'a, T: Send + Sync> Deref for BatchSender<'a, T> {
    type Target = BatchSenderInner<'a, T>;

    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

impl<'a, T: Send + Sync> BatchSender<'a, T> {
    pub fn new(threshold: usize, tx: CountingVecSender<'a, T>) -> Self {
        Self(Arc::new(BatchSenderInner {
            buffer: SegQueue::new(),
            flush_lock: Mutex::new(()),
            threshold,
            tx,
        }))
    }
}

pub struct BatchSenderInner<'a, T: Send + Sync> {
    buffer: SegQueue<T>,
    threshold: usize,
    flush_lock: Mutex<()>,
    tx: CountingVecSender<'a, T>,
}

impl<'a, T: Send + Sync> Debug for BatchSenderInner<'a, T> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("BatchSenderInner")
            .field("buffer", &self.buffer)
            .field("threshold", &self.threshold)
            .field("flush_lock", &self.flush_lock)
            .finish()
    }
}

impl<'a, T: Send + Sync> BatchSenderInner<'a, T> {
    pub fn add(&self, item: T) -> Result<(), flume::SendError<Vec<T>>> {
        self.buffer.push(item);

        if self.buffer.len() >= self.threshold {
            self.flush(false)?;
        }

        Ok(())
    }

    pub fn flush(&self, wait_for_lock: bool) -> Result<(), flume::SendError<Vec<T>>> {
        // Don't flush from multiple threads at the same time. It doesn't really hurt anything
        // since we're not strict about batch sizes, but leads to less efficiency.
        let lock = if wait_for_lock {
            self.flush_lock.lock()
        } else {
            match self.flush_lock.try_lock() {
                Some(lock) => lock,
                None => return Ok(()),
            }
        };

        let output_len = self.buffer.len();
        if output_len == 0 {
            return Ok(());
        }

        let mut output = Vec::with_capacity(self.buffer.len());
        while output.len() < output_len && !self.buffer.is_empty() {
            if let Some(item) = self.buffer.pop() {
                output.push(item);
            }
        }

        drop(lock);

        self.tx.send(output)?;

        Ok(())
    }
}

impl<'a, T: Send + Sync> Drop for BatchSenderInner<'a, T> {
    fn drop(&mut self) {
        self.flush(true).ok();
    }
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::AtomicU64;

    use super::*;

    #[test]
    fn sends_nothing() {
        let (tx, rx) = flume::unbounded();
        let cnt = AtomicU64::new(0);
        let tx = CountingVecSender { count: &cnt, tx };

        let sender = BatchSender::<i32>::new(5, tx);

        drop(sender);

        assert!(rx.is_empty());
    }

    #[test]
    fn sends_exact_batch_size() {
        let (tx, rx) = flume::unbounded();
        let cnt = AtomicU64::new(0);
        let tx = CountingVecSender { count: &cnt, tx };

        let sender = BatchSender::new(5, tx);

        for i in 1..=5 {
            sender.add(i).unwrap();
        }

        drop(sender);

        assert_eq!(rx.len(), 1);
        let output = rx.recv().unwrap();

        assert_eq!(output, vec![1, 2, 3, 4, 5]);
    }

    #[test]
    fn sends_multiple_batches() {
        let (tx, rx) = flume::unbounded();
        let cnt = AtomicU64::new(0);
        let tx = CountingVecSender { count: &cnt, tx };

        let sender = BatchSender::new(2, tx);

        for i in 1..=5 {
            sender.add(i).unwrap();
        }

        assert_eq!(rx.len(), 2);

        drop(sender);

        assert_eq!(rx.len(), 3);

        let output = rx.into_iter().collect::<Vec<_>>();

        assert_eq!(output, vec![vec![1, 2], vec![3, 4], vec![5],]);
    }

    #[test]
    fn ends_at_batch_size_multiple() {
        let (tx, rx) = flume::unbounded();
        let cnt = AtomicU64::new(0);
        let tx = CountingVecSender { count: &cnt, tx };

        let sender = BatchSender::new(3, tx);

        for i in 1..=6 {
            sender.add(i).unwrap();
        }

        assert_eq!(rx.len(), 2);

        drop(sender);

        assert_eq!(rx.len(), 2);

        let output = rx.into_iter().collect::<Vec<_>>();

        assert_eq!(output, vec![vec![1, 2, 3], vec![4, 5, 6],]);
    }

    #[test]
    fn multiple_threads() {
        let (tx, rx) = flume::unbounded();
        let cnt = AtomicU64::new(0);
        let tx = CountingVecSender { count: &cnt, tx };
        let sender = BatchSender::new(3, tx);

        let num_threads = 10;
        let start_flag = std::sync::Barrier::new(num_threads);
        std::thread::scope(|scope| {
            for _ in 0..num_threads {
                scope.spawn(|| {
                    start_flag.wait();
                    for i in 0..6 {
                        sender.add(i).unwrap();
                    }
                });
            }
        });

        drop(sender);

        // Make sure all the values came through. We don't care about the order.
        let mut seen = Vec::new();
        seen.resize(6, 0);

        for batch in rx {
            for item in batch {
                seen[item] += 1;
            }
        }

        // All the values should come through.
        assert_eq!(seen, vec![10, 10, 10, 10, 10, 10]);
    }
}
