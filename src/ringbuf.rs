use std::cmp::min;
use std::{convert::TryFrom, time::Instant};
use std::iter::Iterator;
use log::warn;

const BUFSIZE: usize = 5;

#[derive(Copy, Clone, Debug)]
pub enum Ping {
    None,
    Sent(Instant),
    Received(u128),
}

#[derive(Debug)]
pub struct RingBuffer {
    data: [Ping; BUFSIZE],
    start_index: usize,
    len: usize,
}

pub struct RingBufferIter<'a> {
    buf: &'a RingBuffer,
    index: usize
}

impl RingBuffer {
    pub fn new() -> RingBuffer {
        RingBuffer {
            data: [Ping::None; BUFSIZE],
            start_index: 0,
            len: 0,
        }
    }

    pub fn get_start_index(&self) -> usize {
        return self.start_index;
    }

    pub fn len(&self) -> usize {
        return self.len;
    }

    pub fn get_next_index(&self) -> u64 {
        u64::try_from(self.start_index + self.len).unwrap()
    }

    pub fn sent(&mut self, time: Instant) {
        let i = self.start_index + self.len;
        self.data[i % BUFSIZE] = Ping::Sent(time);
        if self.len < BUFSIZE {
            self.len += 1;
        } else {
            self.start_index += 1;
        }
    }

    pub fn received(&mut self, id: u64) {
        let id_usize = usize::try_from(id).unwrap();
        if id_usize >= self.start_index + self.len {
            panic!("Received a ping we haven't sent yet 👻");
        } else if id_usize >= self.start_index {
            match self.data[id_usize % BUFSIZE] {
                Ping::None => panic!(),
                Ping::Sent(time) => {
                    self.data[id_usize % BUFSIZE] = Ping::Received(time.elapsed().as_millis());
                }
                Ping::Received(_) => {
                    warn!("Received duplicate response");
                }
            }
        }
    }

    pub fn get_data(&self) -> Vec<Ping> {
        let mut vec = Vec::with_capacity(self.len);
        vec.extend_from_slice(&self.data[self.start_index % BUFSIZE..min(self.start_index + self.len, BUFSIZE)]);
        vec.extend_from_slice(&self.data[..self.start_index % BUFSIZE]);
        vec
    }

    pub fn iter(&self) -> RingBufferIter<'_> {
        return self.iter_from(self.start_index);
    }

    pub fn iter_from(&self, index: usize) -> RingBufferIter<'_> {
        return RingBufferIter {
            buf: self,
            index
        }
    }
}

impl std::ops::Deref for RingBuffer {
    type Target = [Ping];
    fn deref(&self) -> &[Ping] {
        &self.data
    }
}

impl std::ops::Index<usize> for RingBuffer {
    type Output = Ping;

    fn index(&self, i: usize) -> &Self::Output {
        if i < self.start_index || i - self.start_index > self.len {
            panic!("Index out of range");
        }
        return &self.data[usize::try_from(i % BUFSIZE).unwrap()];
    }
}

impl std::ops::IndexMut<usize> for RingBuffer {
    fn index_mut(&mut self, i: usize) -> &mut Self::Output {
        if i < self.start_index || i - self.start_index > self.len {
            panic!("Index out of range");
        }
        return &mut self.data[usize::try_from(i % BUFSIZE).unwrap()];
    }
}

impl Iterator for RingBufferIter<'_> {
    type Item = Ping;
    
    fn next(&mut self) -> Option<<Self as Iterator>::Item> {
        if self.index < self.buf.start_index + self.buf.len {
            self.index += 1;
            Some(self.buf[self.index - 1])
        } else {
            None
        }
    }
}
