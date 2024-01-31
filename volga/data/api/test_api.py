import datetime
import unittest

from volga.data.api.dataset.dataset import dataset, field, Dataset
from volga.data.api.dataset.pipeline import pipeline


class TestApi(unittest.TestCase):

    def test_dataset(self):

        @dataset
        class User:
            user_id: str = field(key=True)
            timestamp: datetime.datetime = field(timestamp=True)
            name: str

        assert isinstance(User, Dataset)
        fields = User._fields
        key_fields = User._key_fields
        timestamp_field = User._timestamp_field

        assert len(fields) == 3
        assert key_fields == ['user_id']
        assert timestamp_field == 'timestamp'

    def test_pipline(self):
        @dataset
        class User:
            user_id: str = field(key=True)
            name: str
            timestamp: datetime.datetime = field(timestamp=True)

        @dataset
        class Order:
            user_id: str = field(key=True)
            product_id: str = field(key=True)
            product_name: str
            timestamp: datetime.datetime = field(timestamp=True)


        @dataset
        class UserOrderInfo:
            user_id: str = field(key=True)
            product_id: str = field(key=True)
            product_name: str
            user_name: str
            timestamp: datetime.datetime = field(timestamp=True)

            @pipeline(inputs=[User, Order])
            def gen(cls, users: Dataset, orders: Dataset):
                return users.join(orders, on=['user_id'])

        assert len(UserOrderInfo._pipeline.inputs) == 2


if __name__ == '__main__':
    t = TestApi()
    t.test_dataset()
    t.test_pipline()