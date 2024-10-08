use std::{collections::{HashMap, VecDeque}, sync::{atomic::{AtomicBool, AtomicI32, Ordering}, Arc, Mutex, RwLock}, thread::JoinHandle};

use super::{buffer_utils::{get_buffer_id, new_buffer_drop_meta}, channel::{AckMessage, Channel}, io_loop::{Bytes, IOHandler, IOHandlerType}, metrics::{MetricsRecorder, NUM_BUFFERS_RECVD, NUM_BYTES_RECVD, NUM_BYTES_SENT}, sockets::SocketMetadata};
use crossbeam::{channel::{bounded, unbounded, Receiver, Sender}, queue::ArrayQueue};
use pyo3::{pyclass, pymethods};
use serde::{Deserialize, Serialize};

// const DEFAULT_OUTPUT_QUEUE_SIZE: usize = 10;

#[derive(Serialize, Deserialize, Clone)]
#[pyclass(name="RustDataReaderConfig")]
pub struct DataReaderConfig {
    output_queue_size: usize
}

#[pymethods]
impl DataReaderConfig { 
    #[new]
    pub fn new(output_queue_size: usize) -> Self {
        DataReaderConfig{
            output_queue_size
        }
    }
}

pub struct DataReader {
    name: String,
    job_name: String,
    channels: Vec<Channel>,

    send_chans: Arc<RwLock<HashMap<String, (Sender<Box<Bytes>>, Receiver<Box<Bytes>>)>>>,
    recv_chans: Arc<RwLock<HashMap<String, (Sender<Box<Bytes>>, Receiver<Box<Bytes>>)>>>,
    out_queue: Arc<Mutex<VecDeque<Box<Bytes>>>>,

    // TODO only one thread actually modifies this, can we simplify?
    watermarks: Arc<RwLock<HashMap<String, Arc<AtomicI32>>>>,
    out_of_order_buffers: Arc<RwLock<HashMap<String, Arc<RwLock<HashMap<i32, Box<Bytes>>>>>>>,

    metrics_recorder: Arc<MetricsRecorder>,

    running: Arc<AtomicBool>,
    dispatcher_thread_handle: Arc<ArrayQueue<JoinHandle<()>>>, // array queue so we do not mutate DataReader and kepp ownership

    config: Arc<DataReaderConfig>
}

impl DataReader {

    pub fn new(name: String, job_name: String, data_reader_config: DataReaderConfig, channels: Vec<Channel>) -> DataReader {
        let n_channels = channels.len();
        let mut send_chans = HashMap::with_capacity(n_channels);
        let mut recv_chans = HashMap::with_capacity(n_channels);
        let mut watermarks = HashMap::with_capacity(n_channels);
        let mut out_of_order_buffers = HashMap::with_capacity(n_channels);

        for ch in &channels {
            // TODO making recv_chans bounded drops throughput 10x, why?
            send_chans.insert(ch.get_channel_id().clone(), unbounded());
            recv_chans.insert(ch.get_channel_id().clone(), unbounded()); 
            watermarks.insert(ch.get_channel_id().clone(), Arc::new(AtomicI32::new(-1)));
            out_of_order_buffers.insert(ch.get_channel_id().clone(), Arc::new(RwLock::new(HashMap::new())));   
        }

        // parse config

        DataReader{
            name: name.clone(),
            job_name: job_name.clone(),
            channels,
            send_chans: Arc::new(RwLock::new(send_chans)),
            recv_chans: Arc::new(RwLock::new(recv_chans)),
            out_queue: Arc::new(Mutex::new(VecDeque::with_capacity(data_reader_config.output_queue_size))),
            watermarks: Arc::new(RwLock::new(watermarks)),
            out_of_order_buffers: Arc::new(RwLock::new(out_of_order_buffers)),
            metrics_recorder: Arc::new(MetricsRecorder::new(name.clone(), job_name.clone())),
            running: Arc::new(AtomicBool::new(false)),
            dispatcher_thread_handle: Arc::new(ArrayQueue::new(1)),
            config: Arc::new(data_reader_config),
        }
    }

    pub fn read_bytes(&self) -> Option<Box<Bytes>> {
        // TODO set limit for backpressure
        let mut locked_out_queue = self.out_queue.lock().unwrap();
        let b = locked_out_queue.pop_front();
        if !b.is_none() {
            let b = b.unwrap();
            Some(b)
        } else {
            None
        }
    }

    fn send_ack(channel_id: &String, buffer_id: u32, sender: Sender<Box<Bytes>>, metrics_recorder: Arc<MetricsRecorder>) {
        // we assume ack channels are unbounded
        let ack = AckMessage{channel_id: channel_id.clone(), buffer_id};
        let b = ack.ser();
        let size = b.len();
        sender.send(b).unwrap();
        metrics_recorder.inc(NUM_BYTES_SENT, channel_id, size as u64);
    }
}

impl IOHandler for DataReader {
    
    fn get_name(&self) -> String {
        self.name.clone()
    }

    fn get_handler_type(&self) -> IOHandlerType {
        IOHandlerType::DataReader
    }

    fn get_channels(&self) -> &Vec<Channel> {
        &self.channels
    }

    fn get_send_chan(&self, sm: &SocketMetadata) -> (Sender<Box<Bytes>>, Receiver<Box<Bytes>>) {
        let hm = &self.send_chans.read().unwrap();
        let v = hm.get(&sm.channel_id).unwrap();
        v.clone()
    }

    fn get_recv_chan(&self, sm: &SocketMetadata) -> (Sender<Box<Bytes>>, Receiver<Box<Bytes>>) {
        let hm = &self.recv_chans.read().unwrap();
        let v = hm.get(&sm.channel_id).unwrap();
        v.clone()
    }

    fn start(&self) {
        // start dispatcher thread: takes message from channels, in shared out_queue
        self.running.store(true, Ordering::Relaxed);
        self.metrics_recorder.start();

        let this_runnning = self.running.clone();
        let this_recv_chans = self.recv_chans.clone();
        let this_send_chans = self.send_chans.clone();
        let this_out_queue = self.out_queue.clone();
        let this_watermarks = self.watermarks.clone();
        let this_out_of_order_buffers = self.out_of_order_buffers.clone();
        let this_metrics_recorder = self.metrics_recorder.clone();
        let this_config = self.config.clone();

        let f = move || {

            while this_runnning.load(Ordering::Relaxed) {
                
                let locked_recv_chans = this_recv_chans.read().unwrap();
                let locked_send_chans = this_send_chans.read().unwrap();
                let locked_watermarks = this_watermarks.read().unwrap();
                let locked_out_of_order_buffers = this_out_of_order_buffers.read().unwrap();
                for channel_id in locked_recv_chans.keys() {
                    let mut locked_out_queue = this_out_queue.lock().unwrap();
                    if locked_out_queue.len() == this_config.output_queue_size {
                        // full
                        drop(locked_out_queue);
                        continue
                    }
                    let recv_chan = locked_recv_chans.get(channel_id).unwrap();
                    let receiver = recv_chan.1.clone();

                    let b = receiver.try_recv();
                    if b.is_ok() {
                        let b = b.unwrap();
                        let size = b.len();
                        this_metrics_recorder.inc(NUM_BUFFERS_RECVD, channel_id, 1);
                        this_metrics_recorder.inc(NUM_BYTES_RECVD, channel_id, size as u64);
                        let buffer_id = get_buffer_id(b.clone());

                        let wm = locked_watermarks.get(channel_id).unwrap().load(Ordering::Relaxed);
                        if buffer_id as i32 <= wm {
                            // drop and resend ack
                            let send_chan = locked_send_chans.get(channel_id).unwrap();
                            let sender = send_chan.0.clone();
                            Self::send_ack(channel_id, buffer_id, sender, this_metrics_recorder.clone());
                        } else {
                            // We don't want out_of_order to grow infinitely and should put a limit on it,
                            // however in theory it should not happen - sender will ony send maximum of it's buffer queue size
                            // before receiving ack and sending more (which happens only after all _out_of_order is processed)
                            let locked_out_of_orders = locked_out_of_order_buffers.get(channel_id).unwrap();
                            let mut locked_out_of_order = locked_out_of_orders.write().unwrap(); 
                            
                            if locked_out_of_order.contains_key(&(buffer_id as i32)) {
                                // duplocate
                                let send_chan = locked_send_chans.get(channel_id).unwrap();
                                let sender = send_chan.0.clone();
                                Self::send_ack(channel_id, buffer_id, sender, this_metrics_recorder.clone());
                            } else {
                                locked_out_of_order.insert(buffer_id as i32, b.clone());
                                let mut next_wm = wm + 1;
                                while locked_out_of_order.contains_key(&next_wm) {
                                    if locked_out_queue.len() == this_config.output_queue_size {
                                        // full
                                        break;
                                    }

                                    let stored_b = locked_out_of_order.get(&next_wm).unwrap();
                                    let stored_buffer_id = get_buffer_id(stored_b.clone());
                                    let payload = new_buffer_drop_meta(stored_b.clone());

                                    locked_out_queue.push_back(payload); 

                                    // send ack
                                    let send_chan = locked_send_chans.get(channel_id).unwrap();
                                    let sender = send_chan.0.clone();
                                    Self::send_ack(channel_id, stored_buffer_id, sender, this_metrics_recorder.clone());
                                    locked_out_of_order.remove(&next_wm);
                                    next_wm += 1;
                                }
                                locked_watermarks.get(channel_id).unwrap().store(next_wm - 1, Ordering::Relaxed);
                            }
                        }
                    }
                }
            }
        };

        let name = &self.name;
        let thread_name = format!("volga_{name}_dispatcher_thread");
        self.dispatcher_thread_handle.push(std::thread::Builder::new().name(thread_name).spawn(f).unwrap()).unwrap();
    }

    fn close (&self) {
        self.running.store(false, Ordering::Relaxed);
        let handle = self.dispatcher_thread_handle.pop();
        handle.unwrap().join().unwrap();
        self.metrics_recorder.close();
    }
}