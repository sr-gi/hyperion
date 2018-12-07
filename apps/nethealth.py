import pickle
import threading
import datetime
from StringIO import StringIO
from binascii import hexlify
from getopt import getopt
from sys import argv
from hyperion import *
from apps import *
from copy import deepcopy
from multiprocessing.connection import Client
from bitcoin_tools.utils import change_endianness
from bitcoin.core.serialize import SerializationTruncationError


class PropInfo:
    """
    Class to store all the information related to the received inventory messages, both for transactions and blocks.
    """

    def __init__(self, node_ip, init_time, init_peers, data_type):
        self.first_peer = node_ip
        self.init_time = init_time
        self.init_peers = init_peers
        self.type = data_type
        self.prop_peers = []
        self.half_prop_time = None


def wipe_stale_data(txs_fout_name, blocks_fout_name, debug):
    """
    Wipe data that has become stale. Data is flagged as stale if after some time has been elapsed (defined by WIPE_TIME)
    the transaction or block has not reached the target propagation (defined by TARGET). Wiped data will be also logged
    in disk. Data will be checked periodically (defined by WIPE_DELAY).

    :param txs_fout_name: File name where to store logs about wiped transactions.
    :type txs_fout_name: str
    :param blocks_fout_name: File name where to store logs about wiped blocks.
    :type blocks_fout_name: str
    :param debug: Whether we are in debug mode or not (more logs are displayed on debug).
    :type debug: bool
    :return: None
    :rtype: None
    """

    global received_invs

    while True:
        t = time.time()
        # Check if the transaction/block  has become stale, either because the propagation hasn't reached the target or
        # because coinscope hasn't realised about it (due to disconnections / inconsistent connections).
        with lock:
            received_invs_copy = deepcopy(received_invs)
        for inv_id in received_invs.keys():
            # Check has long the data has been in memory.
            elapsed_t = t - received_invs[inv_id].init_time
            if elapsed_t >= WIPE_TIME:
                # Compute the propagation reached.
                reached_prop = len(received_invs[inv_id].prop_peers) / float(len(received_invs[inv_id].init_peers)) * 100
                time_human = datetime.fromtimestamp(t).strftime('%Y-%m-%d %H:%M:%S')
                if debug:
                    print "%s wiping stale %s %s. Partial data: [%d%%: %.2f seconds]" % \
                          (time_human, received_invs[inv_id].type, inv_id, reached_prop, elapsed_t)

                # Log data depending on the type
                if received_invs[inv_id].type == 'tx':
                    fout = open(txs_fout_name, 'a')
                else:
                    fout = open(blocks_fout_name, 'a')

                fout.write('%d,%s,%s,%.2f,%.2f\n' % (received_invs[inv_id].init_time, inv_id,
                                                     received_invs[inv_id].first_peer, reached_prop, elapsed_t))
                fout.close()

                # Wipe data from dicts.
                received_invs.pop(inv_id)
        with lock:
            if received_invs_copy != received_invs:
                pickle.dump((received_invs_copy, received_invs, t), open('log_wiped_data.pkl', 'a'), protocol=2)
        # Wait until next interval.
        time.sleep(WIPE_DELAY)


def check_received_invs(txs_fout_name, blocks_fout_name, debug):
    """
    Parses the received inventory messages by reading from the coinscope daemon. Once parsed, inv messages are split in
    transactions and blocks.

    :param txs_fout_name: Transaction output file name for disk storage logs.
    :type txs_fout_name: str
    :param blocks_fout_name: Block output file name for disk storage logs.
    :type blocks_fout_name: str
    :param debug: Whether we are in debug mode or not (more logs are displayed on debug).
    :type debug: bool
    :return: None
    :rtype: None
    """

    # Connect to coinscoped and get logs from it
    conn = Client((HOST, PORT))
    conn.send('getlogs')

    while True:
        log = conn.recv()
        try:
            # Deserialize Bitcoin message
            msg = MsgSerializable.stream_deserialize(StringIO(log.bitcoin_msg))
        except ValueError:
            continue
        except SerializationTruncationError as e:
            print e
            continue

        # Parse only inv messages.
        if msg is not None and msg.command == 'inv':
            # Invs type can be either 1 (MSG_TX) or 2 (MSG_BLOCK)
            txs = [change_endianness(hexlify(i.hash)) for i in msg.inv if i.type == MSG_TX]
            manage_invs(txs, 'tx', log.handle_id, txs_fout_name, debug)
            blocks = [change_endianness(hexlify(i.hash)) for i in msg.inv if i.type == MSG_BLOCK]
            manage_invs(blocks, 'block', log.handle_id, blocks_fout_name, debug)


def manage_invs(invs, data_type, peer_id, fout_name, debug):
    """
    Manages the reception of inventory messages in order to decide when the target propagation has been reached.

    :param invs: List of inventory messages representing the inventory vector contained in the last inv message
    received.
    :type   invs: list
    :param data_type: Type of the data to be analysed. Either transactions (tx) or blocks (block).
    :type data_type: str
    :param peer_id: Coinscope id of the peer who sent us the inv message.
    :type peer_id: int
    :param fout_name: Output file name to log propagation data.
    :type fout_name: str
    :param debug: Whether we are in debug mode or not (more logs are displayed on debug).
    :type debug: bool
    :return:None
    :rtype: None
    """

    global received_invs
    global graphite_list

    # Get list of peers from coinscoped
    conn = Client((HOST, PORT))
    conn.send('getpeers')
    peers = conn.recv()

    with lock:
        for inv_id in invs:
            # Check that the peer is still in the peer list.
            peer = peers.get(peer_id)
            if inv_id not in received_invs:
                if peer:
                    # If it's the first time we heard about the tx/block we create a data structure representing it.
                    received_invs[inv_id] = PropInfo(peer, time.time(), peers.values(), data_type)
                    if debug:
                        time_human = datetime.now().strftime('%Y-%m-%d %H:%M:%S')
                        print "%s new inv from node %s: %s %s" % (time_human, peers[peer_id], data_type, inv_id)
            elif peer in received_invs[inv_id].init_peers:
                # Otherwise we update the existing one and add the peer to the propagated peers list
                received_invs[inv_id].prop_peers.append(peer)

                if len(received_invs[inv_id].prop_peers) >= HALF_TARGET * len(received_invs[inv_id].init_peers) and \
                        received_invs[inv_id].half_prop_time is None:
                    # If the half propagation has been reached, we update the half_time value.
                    received_invs[inv_id].half_prop_time = time.time() - received_invs[inv_id].init_time
                    if debug:
                        time_human = datetime.now().strftime('%Y-%m-%d %H:%M:%S')
                        print "%s propagation: %d%% network reached for %s %s in %.2f seconds" % \
                              (time_human, int(HALF_TARGET * 100), data_type, inv_id,
                               received_invs[inv_id].half_prop_time)

                if len(received_invs[inv_id].prop_peers) >= TARGET * len(received_invs[inv_id].init_peers):
                    # If the target propagation has been reached we compute the propagation time, log the data, and flag
                    #  it to be stored in graphite.
                    t = time.time() - received_invs[inv_id].init_time

                    # Check the wipe time has not passed (some data seems not to be properly wiped by the wiping thread)
                    if t <= WIPE_TIME:
                        # Display in terminal.
                        time_human = datetime.now().strftime('%Y-%m-%d %H:%M:%S')
                        print "%s %s %s propagation stats: [%d%%: %.2f seconds] [%d%%: %.2f seconds]" % \
                              (time_human, data_type, inv_id, int(HALF_TARGET * 100),
                               received_invs[inv_id].half_prop_time, int(TARGET * 100), t)

                        # Log it in disk
                        fout = open(fout_name, 'a')
                        fout.write('%d,%s,%s,%.2f,%.2f\n' % (received_invs[inv_id].init_time, inv_id,
                                                             received_invs[inv_id].first_peer,
                                                             received_invs[inv_id].half_prop_time, t))
                        fout.close()

                        # And prepare it to be sent to graphite (depending on the type).
                        if data_type == 'tx':
                            graphite_list.append([GRAPHITE_TX_50, received_invs[inv_id].init_time,
                                                  received_invs[inv_id].half_prop_time])
                            graphite_list.append([GRAPHITE_TX_FULL, received_invs[inv_id].init_time, t])
                        else:
                            graphite_list.append([GRAPHITE_BLOCK_50, received_invs[inv_id].init_time,
                                                  received_invs[inv_id].half_prop_time])
                            graphite_list.append([GRAPHITE_BLOCK_FULL, received_invs[inv_id].init_time, t])

                        graphite_list.append([GRAPHITE_PEER_COUNT, received_invs[inv_id].init_time,
                                              len(received_invs[inv_id].init_peers)])

                    else:
                        # ToDO: This is just a log of data that has not been properly wiped to see if we can figure out
                        # what's going on.
                        f_issue = open('time_issues.csv', 'a')
                        f_issue.write("%d,%s,%s,%.2f,%.2f\n" % (received_invs[inv_id].init_time, inv_id,
                                                                received_invs[inv_id].first_peer,
                                                                received_invs[inv_id].half_prop_time, t))
                        f_issue.close()

                    # Either way, we wipe records from logged transactions.
                    received_invs.pop(inv_id)


def update_graphite():
    """
    Handles the connection with graphite and the update of the lists to be committed. Data is send periodically
    (depending on UPDATE_DELAY).

    :return: None
    :rtype: None
    """

    global graphite_list

    # Open socket with graphite
    sock = socket.socket()
    sock.connect((CARBON_SERVER, CARBON_PICKLE_PORT))

    while True:
        # Build the list of tuples to be committed.
        commit_list = [(graphite_path, (timestamp, value)) for graphite_path, timestamp, value in graphite_list]

        if len(commit_list) > 0:
            time_human = datetime.now().strftime('%Y-%m-%d %H:%M:%S')
            print "%s sending data to graphite." % time_human

            # Send values to graphite.
            package = pickle.dumps(commit_list, protocol=2)
            size = struct.pack('!L', len(package))
            sock.sendall(size)
            sock.sendall(package)

            graphite_list = graphite_list[len(commit_list):]

        time.sleep(UPDATE_DELAY)


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

    GRAPHITE_TX_50 = 'bitcoin_%s.tx_prop_time_50' % net
    GRAPHITE_TX_FULL = 'bitcoin_%s.tx_prop_time_85' % net
    GRAPHITE_BLOCK_50 = 'bitcoin_%s.block_prop_time_50' % net
    GRAPHITE_BLOCK_FULL = 'bitcoin_%s.block_prop_time_85' % net
    GRAPHITE_PEER_COUNT = 'bitcoin_%s.peer_count' % net

    graphite_list = []
    received_invs = dict()
    lock = threading.Lock()

    # Wipe stale inv data.
    t_wipe = threading.Thread(target=wipe_stale_data, args=['wiped_txs.csv', 'wiped_blocks.csv', debug])
    t_wipe.start()

    # Maintain graphite updated.
    t_graph = threading.Thread(target=update_graphite)
    t_graph.start()

    # Check received invs.
    check_received_invs(txs_fout_name='tx_prop_time.csv', blocks_fout_name='blocks_prop_time.csv', debug=debug)