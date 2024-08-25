from typing import List, Dict, Any

from volga.streaming.runtime.network.channel import Channel
from volga.streaming.runtime.network.io_loop import IOLoop
from volga.streaming.runtime.network.local.data_reader import DataReader
from volga.streaming.runtime.network.local.data_writer import DataWriter
import time
import ray
from volga.streaming.runtime.network.network_config import DataWriterConfig, DEFAULT_DATA_WRITER_CONFIG


@ray.remote(max_concurrency=4)
class TestWriter:
    def __init__(
        self,
        writer_id: int,
        job_name: str,
        channels: List[Channel],
        writer_config: DataWriterConfig = DEFAULT_DATA_WRITER_CONFIG,
        delay_s: float = 0,
    ):
        self.channels = channels
        self.io_loop = IOLoop(f'writer_loop_{writer_id}')
        self.delay_s = delay_s
        self.data_writer = DataWriter(
            name=f'test_writer_{writer_id}',
            source_stream_name='0',
            job_name=job_name,
            channels=channels,
            config=writer_config
        )
        self.io_loop.register_io_handler(self.data_writer)

    def start(self, num_threads: int = 1):
        return self.io_loop.start(num_threads)

    def send_items(self, items_per_channel: Dict[str, List[Dict]]):
        index = {channel_id: 0 for channel_id in items_per_channel}
        channel_ids = list(items_per_channel.keys())
        cur_channel_index = 0
        num_sent = 0
        num_to_send = sum(list(map(lambda e: len(e),list(items_per_channel.values()))))
        # round robin send
        while num_sent != num_to_send:
            channel_id = channel_ids[cur_channel_index]
            items = items_per_channel[channel_id]
            i = index[channel_id]
            if i == len(items):
                cur_channel_index = (cur_channel_index + 1)%len(channel_ids)
                continue
            item = items[i]
            succ = self.data_writer.try_write_message(channel_id, item)
            if succ:
                index[channel_id] += 1
                num_sent += 1
            cur_channel_index = (cur_channel_index + 1) % len(channel_ids)

    def close(self):
        self.io_loop.close()


@ray.remote(max_concurrency=4)
class TestReader:
    def __init__(
        self,
        reader_id: int,
        job_name: str,
        channels: List[Channel],
        num_expected: int,
    ):
        self.channels = channels
        self.io_loop = IOLoop(f'reader_loop_{reader_id}')
        self.data_reader = DataReader(
            name=f'test_reader_{reader_id}',
            channels=channels,
            job_name=job_name,
        )
        self.num_expected = num_expected
        self.io_loop.register_io_handler(self.data_reader)
        self.res = []

    def start(self, num_threads: int = 1):
        return self.io_loop.start(num_threads)

    def receive_items(self) -> List[Any]:
        start_ts = time.time()
        while True:
            if time.time() - start_ts > 3000:
                raise RuntimeError('Timeout reading data')

            items = self.data_reader.read_message()
            if items is None:
                time.sleep(0.001)
                continue

            self.res.extend(items)
            if len(self.res) == self.num_expected:
                break
        return self.res

    def get_items(self) -> List:
        return self.res

    def close(self):
        self.io_loop.close()


def start_ray_io_handler_actors(handler_actors: List):
    futs = [h.start.remote() for h in handler_actors]
    ray.get(futs)