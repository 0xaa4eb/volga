use serde::{Deserialize, Serialize};

#[derive(Serialize, Deserialize, PartialEq)]
pub struct ChannelMessage {
    pub key: String,
    pub value: String
}

#[derive(Clone)]
pub enum Channel {
    Local {
        channel_id: String,
        ipc_addr: String
    },
    Remote {
        channel_id: String,
        source_local_ipc_addr: String,
        source_node_ip: String,
        source_node_id: String,
        target_local_ipc_addr: String,
        target_node_ip: String,
        target_node_id: String,
        port: i32,
    }
}

impl Channel {
    pub fn get_channel_id(&self) -> &String {
        match &self {
            Channel::Local { channel_id, ..} => {
                channel_id
            },
            Channel::Remote { channel_id, ..} => {
                channel_id
            }
        }
    }
}
