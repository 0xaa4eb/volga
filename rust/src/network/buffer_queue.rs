<<<<<<< HEAD
use std::{collections::{HashMap, HashSet, VecDeque}, sync::{atomic::{AtomicU64, AtomicU8, Ordering}, Arc, Mutex, RwLock}};

use super::{buffer_utils::get_buffer_id, channel::{AckMessage, Channel}, io_loop::Bytes};
=======
use std::{collections::{HashMap, HashSet, VecDeque}, sync::{atomic::{AtomicU32, AtomicU8, Ordering}, Arc, Mutex, RwLock}};

use super::{buffer_utils::{get_buffer_id, new_buffer_with_meta}, channel::{AckMessage, Channel}, io_loop::Bytes};
>>>>>>> 85a48ff ([Rustify Network] Acks WIP)


pub const MAX_BUFFERS_PER_CHANNEL: usize = 10;

pub struct BufferQueue {
    in_queues: Arc<RwLock<HashMap<String, Arc<Mutex<VecDeque<Box<Bytes>>>>>>>,
    schedule_index: Arc<RwLock<HashMap<String, Arc<AtomicU8>>>>,
<<<<<<< HEAD
    buffer_id_seq: Arc<RwLock<HashMap<String, Arc<AtomicU64>>>>,
=======
    buffer_ids_seq: Arc<RwLock<HashMap<String, Arc<AtomicU32>>>>,
>>>>>>> 85a48ff ([Rustify Network] Acks WIP)
    pop_requests: Arc<RwLock<HashMap<String, Arc<Mutex<HashSet<u32>>>>>>
}

impl BufferQueue {
    pub fn new(channels: Vec<Channel>) -> BufferQueue {
        let n_channels = channels.len();
        let mut in_queues = HashMap::with_capacity(n_channels);
        let mut schedule_index = HashMap::with_capacity(n_channels);
<<<<<<< HEAD
        let mut buffer_id_seq = HashMap::with_capacity(n_channels);
=======
        let mut buffer_ids_seq = HashMap::with_capacity(n_channels);
>>>>>>> 85a48ff ([Rustify Network] Acks WIP)
        let mut pop_requests = HashMap::with_capacity(n_channels);

        for ch in channels {
            in_queues.insert(ch.get_channel_id().clone(), Arc::new(Mutex::new(VecDeque::with_capacity(MAX_BUFFERS_PER_CHANNEL))));
            schedule_index.insert(ch.get_channel_id().clone(), Arc::new(AtomicU8::new(0)));
<<<<<<< HEAD
            buffer_id_seq.insert(ch.get_channel_id().clone(), Arc::new(AtomicU64::new(0)));
=======
            buffer_ids_seq.insert(ch.get_channel_id().clone(), Arc::new(AtomicU32::new(0)));
>>>>>>> 85a48ff ([Rustify Network] Acks WIP)
            pop_requests.insert(ch.get_channel_id().clone(), Arc::new(Mutex::new(HashSet::new())));
        }

        BufferQueue{
            in_queues: Arc::new(RwLock::new(in_queues)),
            schedule_index: Arc::new(RwLock::new(schedule_index)),
<<<<<<< HEAD
            buffer_id_seq: Arc::new(RwLock::new(buffer_id_seq)),
=======
            buffer_ids_seq: Arc::new(RwLock::new(buffer_ids_seq)),
>>>>>>> 85a48ff ([Rustify Network] Acks WIP)
            pop_requests: Arc::new(RwLock::new(pop_requests))
        }
    }

    pub fn try_push(&self, channel_id: &String, b: Box<Bytes>) -> bool {
        let locked_queues = self.in_queues.read().unwrap();
        let mut locked_queue = locked_queues.get(channel_id).unwrap().lock().unwrap();

        if locked_queue.len() == MAX_BUFFERS_PER_CHANNEL {
            return false;
        }

<<<<<<< HEAD
        // TODO set buffer metadata

        locked_queue.push_back(b.clone());
=======
        // set buffer metadata
        let locked_buffer_ids_seq = self.buffer_ids_seq.read().unwrap();
        let buffer_id = locked_buffer_ids_seq.get(channel_id).unwrap().fetch_add(1, Ordering::Relaxed);
        let b_with_meta = new_buffer_with_meta(b, channel_id.clone(), buffer_id);
        locked_queue.push_back(b_with_meta);
>>>>>>> 85a48ff ([Rustify Network] Acks WIP)
        true
    }

    // returns value from queue at schedule index without popping
    pub fn schedule_next(&self, channel_id: &String) -> Option<Box<Bytes>> {
        let locked_queues = self.in_queues.read().unwrap();
        let locked_queue = locked_queues.get(channel_id).unwrap().lock().unwrap();
        if locked_queue.len() == 0 {
            return None;
        }

        let locked_index = self.schedule_index.read().unwrap();
        let schedule_index = locked_index.get(channel_id).unwrap();
<<<<<<< HEAD
=======
        let index = schedule_index.load(Ordering::Relaxed);
        if index >= locked_queue.len() as u8 {
            return None;
        }
>>>>>>> 85a48ff ([Rustify Network] Acks WIP)
        Some(locked_queue.get(schedule_index.fetch_add(1, Ordering::Relaxed) as usize).unwrap().clone())
    }

    // submits pop request, performs pop only for in-order requests
<<<<<<< HEAD
    pub fn request_pop(&self, channel_id: &String, ack: AckMessage) {
        let locked_pop_requests = self.pop_requests.read().unwrap();
        let mut locked_pop_request = locked_pop_requests.get(channel_id).unwrap().lock().unwrap();
        locked_pop_request.insert(ack.buffer_id);
=======
    pub fn request_pop(&self, channel_id: &String, buffer_id: u32) {
        let locked_pop_requests = self.pop_requests.read().unwrap();
        let mut locked_pop_request = locked_pop_requests.get(channel_id).unwrap().lock().unwrap();
        locked_pop_request.insert(buffer_id);
>>>>>>> 85a48ff ([Rustify Network] Acks WIP)

        let locked_index = self.schedule_index.read().unwrap();
        let schedule_index = locked_index.get(channel_id).unwrap();

        let locked_queues = self.in_queues.read().unwrap();
        let mut locked_queue = locked_queues.get(channel_id).unwrap().lock().unwrap();

        while locked_queue.len() != 0 {
            let peek_buffer = locked_queue.get(0).unwrap();
            let peek_buffer_id = get_buffer_id(peek_buffer.clone());
            if locked_pop_request.contains(&peek_buffer_id) {
                locked_queue.pop_front();
                locked_pop_request.remove(&peek_buffer_id);
                schedule_index.fetch_sub(1, Ordering::Relaxed);
<<<<<<< HEAD
=======
            } else {
                break;
>>>>>>> 85a48ff ([Rustify Network] Acks WIP)
            }
        }
    }
}