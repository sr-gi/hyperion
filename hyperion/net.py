from hyperion import *
from StringIO import StringIO
from lib.logger import update_types
from hyperion import shared
import ujson


class PeerInfo:
    def __init__(self, subver, first_seen, coinscope_id):
        self.subver = subver
        self.first_seen = first_seen
        self.last_seen = first_seen
        self.coinscope_id = coinscope_id
        self.last_updated = first_seen  # time when coinscope_id changed for the last time

    def update(self, coinscope_id, subver, last_seen):
        if coinscope_id != self.coinscope_id:
            self.coinscope_id = coinscope_id
            self.last_updated = last_seen
        if self.subver != subver:
            self.subver = subver
        self.last_seen = last_seen

    def tostring(self):
        return "[subver: %s, first_seen: %s, last_seen: %s, coinscope_id: %s, last_updated: %s]" \
               % (self.subver, self.first_seen, self.last_seen, self.coinscope_id, self.last_updated)

    def todict(self):
        return {'subver': self.subver, 'first_seen': self.first_seen, 'last_seen': self.last_seen,
                'coinscope_id': self.coinscope_id, 'last_updated': self.last_updated}


def update_ldb(ldb, peer_id, peer_meta, new):
    prefix = cfg.get('coinscope', 'coin')
    key = prefix + peer_id

    if new:
        coinscope_ids = {peer_meta.last_updated: peer_meta.coinscope_id}
    else:
        prev_data = ldb.get(key)
        coinscope_ids = ujson.loads(prev_data).get('coinscope_ids')
        coinscope_ids[peer_meta.last_updated] = peer_meta.coinscope_id

    value = {'subver': peer_meta.subver, 'first_seen': peer_meta.first_seen, 'last_updated': peer_meta.last_updated,
             'coinscope_ids': coinscope_ids}

    ldb.put(key, ujson.dumps(value))


def manage_connections(avoid_port, debug):
    """
    Manages connections and disconnections from coinscope peers by reading the connection socket. Maintains an updated
    dictionary containing the peers coinscope is connected to at any given time.

    :param avoid_port: Port to avoid depending on the network we are analysing. Peers using that port will be dropped
    from the peers dict. Even though peers from mainnet should not be found in testnet and vice versa, some wrong
    addresses end up being heard by coinscope.
    :type avoid_port: int
    :param debug: Whether we are in debug mode or not (more logs are displayed on debug).
    :type debug: bool
    :return: None
    :rtype: None
    """

    # Open socket connection
    log_sock = socket.socket(socket.AF_UNIX, socket.SOCK_STREAM, 0)
    log_sock.connect(COINSCOPE_PATH + cfg.get('coinscope', 'log_sock'))

    for log in read_logs(log_sock):
        # New connection
        if log.update_type in [1, 2]:
            if str(log.remote_port) != str(avoid_port):
                with shared.peers_lock:
                    shared.peers[log.handle_id] = log.remote_addr + ':' + str(log.remote_port)
                if debug:
                    print "New connection: %s (%d)" % (log.remote_addr + ':' + str(log.remote_port), log.handle_id)
            else:
                if log.handle_id not in shared.ignored_peers:
                    with shared.peers_lock:
                        shared.ignored_peers[log.handle_id] = log.remote_addr + ':' + str(log.remote_port)

        # New disconnection
        elif log.update_type in [4, 8, 16, 64, 128] and log.handle_id in shared.peers.keys():
            with shared.peers_lock:
                shared.peers.pop(log.handle_id)
            if debug:
                print "New disconnection (%s): %s (%d)." % (update_types.str_mapping[log.update_type],
                                                            log.remote_addr + ':' + str(log.remote_port), log.handle_id)


def manage_peers_metadata(debug):
    # Open socket connection
    log_sock = socket.socket(socket.AF_UNIX, socket.SOCK_STREAM, 0)
    log_sock.connect(COINSCOPE_PATH + cfg.get('coinscope', 'bitcoin_msg'))

    try:
        for log in read_logs(log_sock):
            # Get log message.
            try:
                msg = MsgSerializable.stream_deserialize(StringIO(log.bitcoin_msg))
            except ValueError:
                continue

            # Check its type.
            if msg is None:
                continue

            with shared.peers_lock:
                peer = shared.peers.get(log.handle_id)

            if not log.is_sender:
                if msg.command == 'version' and peer:
                    if peer not in shared.peers_meta:
                        shared.peers_meta[peer] = PeerInfo(subver=msg.strSubVer, first_seen=log.timestamp,
                                                           coinscope_id=log.handle_id)

                        update_ldb(shared.ldb, peer, shared.peers_meta[peer], new=True)

                        if debug:
                            print "New peer (%s): %s" % (peer, shared.peers_meta[peer].tostring())
                    else:
                        shared.peers_meta[peer].update(coinscope_id=log.handle_id, subver=msg.strSubVer,
                                                       last_seen=log.timestamp)

                        update_ldb(shared.ldb, peer, shared.peers_meta[peer], new=False)

                        if debug:
                            print "Updating peer metadata (%s): %s" % (peer, shared.peers_meta[peer].tostring())

                else:
                    if peer in shared.peers_meta:
                        shared.peers_meta[peer].last_seen = log.timestamp

                        # if debug:
                        #     print "Updating peer last seen (%s): %s" % (peer, shared.peers_meta[peer].tostring())
                    elif log.handle_id not in shared.ignored_peers and debug:
                        print "Peer not found (%s)." % log.handle_id

    except ValueError:
        # In some cases the timeout can turn out to be negative, making the thread to stop without having stored the
        # logs. If this happens we just stored the logs and terminate.
        pass


