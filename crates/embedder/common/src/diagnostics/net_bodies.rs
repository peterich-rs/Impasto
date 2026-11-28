use std::collections::{HashMap, VecDeque};
use std::sync::Arc;

use impasto::network::BODY_LIMIT;

const MAX_BODIES: usize = 8192;
const NET_BODY_POOL_BYTES: usize = 16 * 1024 * 1024;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub(super) enum BodySide {
    Request,
    Response,
    Socket(u64),
}

#[derive(Default)]
pub(super) struct NetBodyPool {
    used_bytes: usize,
    order: VecDeque<(u64, BodySide)>,
    bodies: HashMap<(u64, BodySide), Arc<[u8]>>,
}

impl NetBodyPool {
    pub(super) fn insert(
        &mut self,
        key: (u64, BodySide),
        mut data: Vec<u8>,
    ) -> Vec<(u64, BodySide)> {
        data.truncate(BODY_LIMIT);
        if data.is_empty() {
            return Vec::new();
        }
        if let Some(previous) = self.bodies.remove(&key) {
            self.used_bytes -= previous.len();
            self.order.retain(|item| *item != key);
        }
        self.used_bytes += data.len();
        self.bodies.insert(key, data.into());
        self.order.push_back(key);
        let mut evicted = Vec::new();
        while self.used_bytes > NET_BODY_POOL_BYTES || self.bodies.len() > MAX_BODIES {
            let Some(oldest) = self.order.pop_front() else {
                break;
            };
            if let Some(body) = self.bodies.remove(&oldest) {
                self.used_bytes -= body.len();
                evicted.push(oldest);
            }
        }
        evicted
    }

    #[cfg(any(test, debug_assertions, feature = "devtools-server"))]
    pub(super) fn get(&mut self, key: (u64, BodySide)) -> Option<Arc<[u8]>> {
        let body = self.bodies.get(&key)?;
        self.order.retain(|item| *item != key);
        self.order.push_back(key);
        Some(Arc::clone(body))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn least_recently_used_body_is_evicted_within_budget() {
        let mut pool = NetBodyPool::default();
        for id in 0..16 {
            assert!(
                pool.insert((id, BodySide::Response), vec![0; BODY_LIMIT])
                    .is_empty()
            );
        }
        assert!(pool.get((0, BodySide::Response)).is_some());
        assert_eq!(
            pool.insert((16, BodySide::Response), vec![9; BODY_LIMIT + 1]),
            vec![(1, BodySide::Response)]
        );
        assert_eq!(pool.used_bytes, NET_BODY_POOL_BYTES);
        assert!(pool.get((1, BodySide::Response)).is_none());
        assert_eq!(
            pool.get((16, BodySide::Response)).unwrap().len(),
            BODY_LIMIT
        );
    }
}
