#[derive(Debug)]
pub struct BatchSender<T> {
    buffer: Vec<T>,
    threshold: usize,
    tx: flume::Sender<Vec<T>>,
}

impl<T> BatchSender<T> {
    pub fn new(threshold: usize, tx: flume::Sender<Vec<T>>) -> Self {
        Self {
            buffer: Vec::with_capacity(threshold),
            threshold,
            tx,
        }
    }

    pub fn add(&mut self, item: T) -> Result<(), flume::SendError<Vec<T>>> {
        self.buffer.push(item);

        if self.buffer.len() >= self.threshold {
            self.flush()?;
        }

        Ok(())
    }

    pub fn add_multiple(&mut self, items: Vec<T>) -> Result<(), flume::SendError<Vec<T>>> {
        self.buffer.extend(items);

        if self.buffer.len() >= self.threshold {
            self.flush()?;
        }

        Ok(())
    }

    pub fn flush(&mut self) -> Result<(), flume::SendError<Vec<T>>> {
        if self.buffer.is_empty() {
            return Ok(());
        }

        let buffer = std::mem::replace(&mut self.buffer, Vec::with_capacity(self.threshold));
        self.tx.send(buffer)?;

        Ok(())
    }
}

impl<T> Drop for BatchSender<T> {
    fn drop(&mut self) {
        self.flush().ok();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sends_nothing() {
        let (tx, rx) = flume::unbounded();

        let sender = BatchSender::<i32>::new(5, tx);

        drop(sender);

        assert!(rx.is_empty());
    }

    #[test]
    fn sends_exact_batch_size() {
        let (tx, rx) = flume::unbounded();

        let mut sender = BatchSender::new(5, tx);

        for i in 1..=5 {
            sender.add(i).unwrap();
        }

        drop(sender);

        assert_eq!(rx.len(), 1);
        let output = rx.recv().unwrap();

        assert_eq!(output, vec![1, 2, 3, 4, 5]);
    }

    #[test]
    fn add_multiple_exact_size() {
        let (tx, rx) = flume::unbounded();

        let mut sender = BatchSender::new(5, tx);

        let input = vec![1, 2, 3, 4, 5];

        for i in 1..=5 {
            sender.add(i).unwrap();
        }

        drop(sender);

        assert_eq!(rx.len(), 1);
        let output = rx.recv().unwrap();

        assert_eq!(input, output);
    }

    #[test]
    fn add_multiple_over_size() {
        let (tx, rx) = flume::unbounded();

        let mut sender = BatchSender::new(5, tx);

        sender.add_multiple(vec![1, 2, 3]).unwrap();
        sender.add_multiple(vec![4, 5, 6]).unwrap();
        assert_eq!(rx.len(), 1);

        drop(sender);
        assert_eq!(rx.len(), 1);

        let output = rx.recv().unwrap();

        assert_eq!(output, vec![1, 2, 3, 4, 5, 6]);
    }

    #[test]
    fn sends_multiple_batches() {
        let (tx, rx) = flume::unbounded();

        let mut sender = BatchSender::new(2, tx);

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

        let mut sender = BatchSender::new(3, tx);

        for i in 1..=6 {
            sender.add(i).unwrap();
        }

        assert_eq!(rx.len(), 2);

        drop(sender);

        assert_eq!(rx.len(), 2);

        let output = rx.into_iter().collect::<Vec<_>>();

        assert_eq!(output, vec![vec![1, 2, 3], vec![4, 5, 6],]);
    }
}
