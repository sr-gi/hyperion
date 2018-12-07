import threading
from getopt import getopt
from sys import argv
from hyperion import *
from hyperion.net import manage_connections, manage_peers_metadata
from hyperion.api import manage_api
from hyperion import shared

if __name__ == '__main__':
    avoid_port = None
    debug = False
    opts, _ = getopt(argv[1:], 'mtd', ['mainnet', 'testnet', 'debug'])

    for opt, arg in opts:
        if opt in ['-m', '--mainnet']:
            avoid_port = TESTNET_PORT
            net = 'mainnet'
        if opt in ['-t', '--testnet']:
            if avoid_port is not None:
                raise Exception("Both mainnet and testnet options selected. Choose just one.")
            else:
                avoid_port = MAINNET_PORT
                net = 'testnet'
        if opt in ['-d', '--debug']:
            debug = True

    if avoid_port is None:
        raise Exception("A network must be chosen.")
    # Get all nodes we are connected to (ip:id).
    node_map = dict(CONNECTOR.get_cxn())

    # Remove all nodes using mainnet port
    for node in node_map.keys():
        ip, port = node.split(':')
        if port == str(avoid_port):
            node_map.pop(node)

    # Init shared variables
    shared.init()

    with shared.peers_lock:
        for ip, hid in node_map.iteritems():
            shared.peers[hid] = ip

    # Maintain an update list of nodes.
    t_connections = threading.Thread(target=manage_connections, args=[avoid_port, debug])
    t_connections.start()

    # Maintain metadata up to date.
    t_meta = threading.Thread(target=manage_peers_metadata, args=[debug])
    t_meta.start()

    # Maintain metadata up to date.
    t_meta = threading.Thread(target=manage_peers_metadata, args=[debug])
    t_meta.start()

    # Serve data on demand
    t_api = threading.Thread(target=manage_api, args=[debug])
    t_api.start()



