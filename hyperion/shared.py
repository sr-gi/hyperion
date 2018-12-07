import threading
import plyvel
from hyperion import COINSCOPE_PATH, cfg


def init():
    global peers, peers_meta, peers_lock, ignored_peers, ldb

    peers = {}
    peers_meta = {}
    peers_lock = threading.Lock()
    ignored_peers = {}

    try:
        ldb = plyvel.DB(COINSCOPE_PATH + cfg.get('coinscope', 'leveldb'), create_if_missing=False)

    except plyvel.Error:
        raise Exception("Can't a leveldb to log peers metadata. Have you created any?")


