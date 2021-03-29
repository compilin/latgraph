use std::marker::PhantomData;
use std::{cmp::min, convert::TryFrom, iter::Iterator, time::Instant};

use log::{warn,debug};

#[derive(Copy, Clone, Debug)]
pub enum Ping {
    Sent(Instant),
    Received(Instant, u128),
}

#[derive(Debug)]
pub struct RingBuffer {
    data: Vec<Ping>,
    start_index: usize,
    capacity: usize,
}

pub struct RingBufferIter<'a, T> {
    buf: &'a RingBuffer,
    index: usize,
    reverse: bool,
    iter_type: PhantomData<T>,
}

#[allow(dead_code)]
impl RingBuffer {
    pub fn new(size: usize) -> RingBuffer {
        RingBuffer {
            data: Vec::with_capacity(size),
            start_index: 0,
            capacity: size,
        }
    }

    pub fn get_start_index(&self) -> usize {
        self.start_index
    }

    pub fn len(&self) -> usize {
        self.data.len()
    }

    pub fn capacity(&self) -> usize {
        self.capacity
    }

    pub fn sent(&mut self, time: Instant) {
        if self.data.len() < self.capacity {
            self.data.push(Ping::Sent(time));
        } else {
            let i = self.start_index + self.data.len();
            self.data[i % self.capacity] = Ping::Sent(time);
            self.start_index += 1;
        }
    }

    pub fn received(&mut self, id: u64, rcv_time: Instant) {
        let id_usize = usize::try_from(id).unwrap();
        if id_usize >= self.start_index + self.data.len() {
            panic!("Received a ping we haven't sent yet 👻");
        } else if id_usize >= self.start_index {
            match self.data[id_usize % self.capacity] {
                Ping::Sent(snd_time) => {
                    let lat = rcv_time.saturating_duration_since(snd_time).as_millis();
                    debug!("Received pong, latency: {}", lat);
                    self.data[id_usize % self.capacity] = Ping::Received(
                        snd_time,
                        lat,
                    );
                }
                Ping::Received(_, _) => {
                    warn!("Received duplicate response");
                }
            }
        }
    }

    pub fn get_data(&self) -> Vec<Ping> {
        let mut vec = Vec::with_capacity(self.data.len());
        vec.extend_from_slice(
            &self.data[self.start_index % self.capacity
                ..min(self.start_index + self.data.len(), self.capacity)],
        );
        vec.extend_from_slice(&self.data[..self.start_index % self.capacity]);
        vec
    }

    pub fn iter(&self) -> RingBufferIter<'_, Ping> {
        RingBufferIter {
            buf: self,
            index: self.start_index,
            reverse: false,
            iter_type: PhantomData,
        }
    }

    pub fn iter_rev(&self) -> RingBufferIter<'_, Ping> {
        RingBufferIter {
            buf: self,
            index: self.start_index + self.data.len(),
            reverse: true,
            iter_type: PhantomData,
        }
    }

    /// Translates "Public" index to index in the buffer
    fn buffer_index(&self, i: usize) -> usize {
        if i < self.start_index || i - self.start_index > self.data.len() {
            panic!("Index out of range");
        }
        i % self.capacity
    }
}

impl std::ops::Index<usize> for RingBuffer {
    type Output = Ping;

    fn index(&self, i: usize) -> &Self::Output {
        &self.data[self.buffer_index(i)]
    }
}

impl std::ops::IndexMut<usize> for RingBuffer {
    fn index_mut(&mut self, i: usize) -> &mut Self::Output {
        let i = self.buffer_index(i);
        &mut self.data[i]
    }
}

impl<'a> RingBufferIter<'a, Ping> {
    pub fn with_index(self) -> RingBufferIter<'a, (usize, Ping)> {
        RingBufferIter {
            buf: self.buf,
            index: self.index,
            reverse: self.reverse,
            iter_type: PhantomData,
        }
    }
}

impl Iterator for RingBufferIter<'_, Ping> {
    type Item = Ping;
    fn next(&mut self) -> Option<<Self as Iterator>::Item> {
        let index =
            if !self.reverse && !self.reverse && self.index < self.buf.start_index + self.buf.len()
            {
                self.index += 1;
                Some(self.index - 1)
            } else if self.reverse && self.index > self.buf.start_index {
                self.index -= 1;
                Some(self.index)
            } else {
                None
            };

        index.map(|i| self.buf[i])
    }
}

impl Iterator for RingBufferIter<'_, (usize, Ping)> {
    type Item = (usize, Ping);
    fn next(&mut self) -> Option<<Self as Iterator>::Item> {
        let index =
            if !self.reverse && !self.reverse && self.index < self.buf.start_index + self.buf.len()
            {
                self.index += 1;
                Some(self.index - 1)
            } else if self.reverse && self.index > self.buf.start_index {
                self.index -= 1;
                Some(self.index)
            } else {
                None
            };

        index.map(|i| (i, self.buf[i]))
    }
}

impl Ping {
    pub fn sent_time(&self) -> Instant {
        match self {
            Ping::Sent(time) => *time,
            Ping::Received(time, _) => *time
        }
    }
}
