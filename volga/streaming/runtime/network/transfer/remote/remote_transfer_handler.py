import os
import time
from collections import deque
from typing import List, Dict, Tuple

import zmq

from volga.streaming.runtime.network.buffer.buffer import get_channel_id
from volga.streaming.runtime.network.stats import Stats, StatsEvent
from volga.streaming.runtime.network.transfer.io_loop import Direction, IOHandler
from volga.streaming.runtime.network.channel import RemoteChannel, ipc_path_from_addr
from volga.streaming.runtime.network.config import NetworkConfig, DEFAULT_NETWORK_CONFIG
from volga.streaming.runtime.network.socket_utils import configure_socket, rcv_no_block, send_no_block, _try_tcp_bind, \
    SocketMetadata, SocketOwner, SocketKind


# base class for remote TransferReceiver and TransferSender
class RemoteTransferHandler(IOHandler):

    def __init__(
        self,
        channels: List[RemoteChannel],
        zmq_ctx: zmq.Context,
        direction: Direction,
        network_config: NetworkConfig = DEFAULT_NETWORK_CONFIG
    ):
        super().__init__(
            channels=channels,
            zmq_ctx=zmq_ctx,
            direction=direction,
            network_config=network_config
        )
        self._is_sender = direction == Direction.SENDER

        # local ipc connections per channel
        self._local_socket_to_ch: Dict[zmq.Socket, str] = {}
        self._local_ch_to_socket: Dict[str, zmq.Socket] = {}

        # remote tcp connection per peer node
        self._remote_socket_to_node: Dict[zmq.Socket, str] = {}
        self._remote_node_to_sock: Dict[str, zmq.Socket] = {}

        self._local_queues: Dict[str, deque] = {c.channel_id: deque() for c in self._channels}
        self._remote_queues: Dict[str, deque] = {}

        self._ch_id_to_node_id = {c.channel_id: c.target_node_id if self._is_sender else c.source_node_id for c in channels}

        self.stats = Stats()

    def create_sockets(self) -> List[Tuple[SocketMetadata, zmq.Socket]]:
        sockets = []
        for channel in self._channels:
            assert isinstance(channel, RemoteChannel)
            if channel.channel_id in self._local_ch_to_socket:
                raise RuntimeError('duplicate channel ids')

            # local socket setup
            local_socket = self._zmq_ctx.socket(zmq.PAIR)
            local_socket_owner = SocketOwner.TRANSFER_LOCAL
            zmq_config = self._network_config.zmq
            if zmq_config is not None:
                configure_socket(local_socket, zmq_config)

            # TODO we should clean up paths on socket deletion
            if self._is_sender:
                ipc_path = ipc_path_from_addr(channel.source_local_ipc_addr)
                os.makedirs(ipc_path, exist_ok=True)
                local_addr = channel.source_local_ipc_addr
                local_socket_kind = SocketKind.CONNECT
            else:
                ipc_path = ipc_path_from_addr(channel.target_local_ipc_addr)
                os.makedirs(ipc_path, exist_ok=True)
                local_addr = channel.target_local_ipc_addr
                local_socket_kind = SocketKind.BIND

            self._local_ch_to_socket[channel.channel_id] = local_socket
            self._local_socket_to_ch[local_socket] = channel.channel_id
            local_socket_metadata = SocketMetadata(
                owner=local_socket_owner,
                kind=local_socket_kind,
                channel_id=channel.channel_id,
                addr=local_addr
            )
            if local_socket_metadata in self._sockets_meta:
                raise
            self._sockets_meta[local_socket_metadata] = local_socket
            sockets.append((local_socket_metadata, local_socket))

            # remote socket setup
            peer_node_id = channel.target_node_id if self._is_sender else channel.source_node_id
            if peer_node_id in self._remote_node_to_sock:
                # already inited for this peer
                continue

            remote_socket = self._zmq_ctx.socket(zmq.PAIR)
            remote_socket_owner = SocketOwner.TRANSFER_REMOTE
            # TODO separate zmq_config for remote and local sockets
            if zmq_config is not None:
                configure_socket(remote_socket, zmq_config)
            if self._is_sender:
                tcp_addr = f'tcp://{channel.target_node_ip}:{channel.port}'
                remote_socket_kind = SocketKind.CONNECT
            else:
                tcp_addr = f'tcp://0.0.0.0:{channel.port}'
                remote_socket_kind = SocketKind.BIND

            remote_socket_metadata = SocketMetadata(owner=remote_socket_owner, kind=remote_socket_kind, channel_id=channel.channel_id, addr=tcp_addr)
            sockets.append((remote_socket_metadata, remote_socket))
            if remote_socket_metadata in self._sockets_meta:
                raise
            self._sockets_meta[remote_socket_metadata] = remote_socket

            self._remote_node_to_sock[peer_node_id] = remote_socket
            self._remote_socket_to_node[remote_socket] = peer_node_id

            # TODO what happens if we reconnect? Do we just drop existing queues?
            self._remote_queues[peer_node_id] = deque()

        return sockets

    def send(self, socket: zmq.Socket):
        if socket in self._local_socket_to_ch:
            channel_id = self._local_socket_to_ch[socket]
            queue = self._local_queues[channel_id]
            stats_event = StatsEvent.ACK_SENT if self._is_sender else StatsEvent.MSG_SENT
            stats_key = channel_id
        elif socket in self._remote_socket_to_node:
            node_id = self._remote_socket_to_node[socket]
            queue = self._remote_queues[node_id]
            stats_event = StatsEvent.MSG_SENT if self._is_sender else StatsEvent.ACK_SENT
            stats_key = node_id
        else:
            raise RuntimeError('Unregistered socket')

        if len(queue) == 0:
            return

        data = queue.popleft()

        # TODO NOBLOCK, handle exceptions, timeouts retries etc.
        t = time.time()
        sent = send_no_block(socket, data)
        if not sent:
            # return data
            queue.insert(0, data)
            # TODO indicate somehow?
            # TODO add delay on retry
            return

        # update stats
        self.stats.inc(stats_event, stats_key)

        remote_or_local = 'remote' if socket in self._remote_socket_to_node else 'local'
        print(f'Sent {remote_or_local} in {time.time() - t}')

    def rcv(self, socket: zmq.Socket):

        t = time.time()
        data = rcv_no_block(socket)
        if data is None:
            # TODO indicate somehow?
            # TODO add delay on retry
            return
        if socket in self._local_socket_to_ch:
            channel_id = self._local_socket_to_ch[socket]
            peer_node_id = self._ch_id_to_node_id[channel_id]
            queue = self._remote_queues[peer_node_id]
            # TODO set limit on the queue size
            queue.append(data)
            self.stats.inc(StatsEvent.MSG_RCVD if self._is_sender else StatsEvent.ACK_RCVD, channel_id)
        elif socket in self._remote_socket_to_node:
            channel_id = get_channel_id(data)
            peer_node_id = self._ch_id_to_node_id[channel_id]
            queue = self._local_queues[channel_id]
            # TODO set limit on the queue size. If a queue is full we have two options:
            #  1) drop the message and count on higher-level retry mechanism to resend it
            #  2) block the whole peer-node channel and count on higher-level backpressure mechanism (Credit based)
            #  we need to test which is better for throughput/latency
            queue.append(data)
            self.stats.inc(StatsEvent.ACK_RCVD if self._is_sender else StatsEvent.MSG_RCVD, peer_node_id)

        else:
            raise RuntimeError('Unregistered socket')

        remote_or_local = 'remote' if socket in self._remote_socket_to_node else 'local'
        print(f'Rcvd {remote_or_local} in {time.time() - t}')

    # TODO we should unbind/disconnect sockets
    def close_sockets(self):
        for s in self._local_socket_to_ch:
            s.close(linger=0)
        for s in self._remote_socket_to_node:
            s.close(linger=0)
