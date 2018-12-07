import threading
import ujson
import numpy as np
from copy import deepcopy
from hyperion import *
from multiprocessing.connection import Listener
from hyperion import shared
from hyperion.logs import hand_logs


def manage_api(debug, host=HOST, port=PORT):
    listener = Listener((host, port))
    while True:
        conn = listener.accept()

        if debug:
            print 'Connection accepted from', listener.last_accepted

        # Maintain metadata up to date.
        t_serve = threading.Thread(target=serve_data, args=[debug, conn, listener.last_accepted])
        t_serve.start()


def serve_data(debug, conn, remote_addr):
    while not conn.closed:
        try:
            msg = conn.recv()

            if type(msg) == tuple:
                command, arg = msg
            else:
                command = msg
                arg = None

            if command == 'getpeer' and arg:
                peer_ip = arg
                peer = shared.peers_meta.get(peer_ip)
                if peer:
                    peer = {peer_ip: peer.todict()}
                    if debug:
                        print 'Serving peer to', remote_addr
                else:
                    peer = {}
                    if debug:
                        print "Can't find peer %s. Reporting to " % peer_ip, remote_addr

                conn.send(peer)

            elif command == 'getpeerold' and type(arg) == list and len(arg) == 2:
                peer_ip = arg[0]
                ts = arg[1]

                ldb_key = cfg.get('coinscope', 'coin') + peer_ip
                peer_ldb = shared.ldb.get(ldb_key)

                if peer_ldb:
                    # ldb values are json encoded
                    peer_ldb = ujson.loads(peer_ldb).get('coinscope_ids')
                    times = np.array(sorted(peer_ldb.keys()))
                    # get all times that fit the threshold
                    times = times[times <= ts]

                    if len(times) > 0:
                        update_time = times[-1]
                        coinscope_id = peer_ldb.get(update_time)

                        # Deep copying the peer by creating anew dict
                        peer_meta = deepcopy(shared.peers_meta.get(peer_ip))
                        peer_meta.coinscope_id = coinscope_id
                        peer_meta.last_updated = str(update_time)
                        peer_meta.last_seen = str(update_time)

                        peer = {peer_ip: peer_meta.todict()}

                    else:
                        peer = {}
                        if debug:
                            print "Can't find peer %s at time %s. Reporting to %s" % (peer_ip, ts, remote_addr)
                else:
                    peer = {}
                    if debug:
                        print "Can't find peer %s. Reporting to " % arg, remote_addr

                conn.send(peer)

            elif command == 'getpeerhistory' and arg:
                peer_ip = arg
                peer_meta = shared.peers_meta.get(peer_ip)
                peer_ldb = shared.ldb.get(cfg.get('coinscope', 'coin') + peer_ip)

                if peer_ldb:
                    # ldb values are json encoded
                    peer_ldb = ujson.loads(peer_ldb)
                    peer_meta = deepcopy(peer_meta)
                    peer_meta.coinscope_id = peer_ldb.get('coinscope_ids')
                    peer = {peer_ip: peer_meta.todict()}

                elif peer_meta:
                    peer = {peer_ip: peer_meta.todict()}

                else:
                    peer = {}
                    if debug:
                        print "Can't find peer %s. Reporting to " % peer_ip, remote_addr

                conn.send(peer)

            elif command == 'getpeers':
                conn.send(shared.peers)

                if debug:
                    print 'Serving peers to', remote_addr

            elif command == 'getpeersinfo':
                # Encode data as dict for better compatibility
                meta_dict = {}
                for peer, meta in shared.peers_meta.iteritems():
                    meta_dict[peer] = meta.todict()

                conn.send(meta_dict)

                if debug:
                    print 'Serving peers meta to', remote_addr

            elif command == 'getpeercount':
                conn.send(len(shared.peers))

            elif command == "getlogs":
                if debug:
                    print 'Serving logs to', remote_addr

                for log in hand_logs(COINSCOPE_PATH + cfg.get('coinscope', 'bitcoin_msg')):
                    conn.send(log)

            elif command == "getnetlogs":
                if debug:
                    print 'Serving net logs to', remote_addr

                for log in hand_logs(COINSCOPE_PATH + cfg.get('coinscope', 'log_sock')):
                    conn.send(log)

        except (IOError, EOFError):
            print 'Disconnecting from', remote_addr
            conn.close()

