from bitcoin.core import *
from bitcoin.net import *
from bitcoin.messages import *
from bitcoin import *
from connector import *
import time
import struct
import logger
from lib import cfg
from bitcoin_tools.core.transaction import TX


# Set coinscope connectors
SelectParams(cfg.get('coinscope', 'network'))


def read_logs_until_signal(logsock, signal_stop):
    logsock.settimeout(None)
    while not signal_stop.is_set():
        try:
            length = logsock.recv(4)
            length, = struct.unpack('>I', length)
            record = ''
            while len(record) < length:
                record += logsock.recv(length - len(record))
            assert (len(record) == length)
        except socket.timeout:
            break
        except struct.error:
            continue
        sid, log_type, timestamp, rest = logger.log.deserialize_part(record)
        try:
            log = logger.type_to_obj[log_type].deserialize(sid, timestamp, rest)
        except KeyError as e:
            print e
            continue
        yield log


# connects to the coinscope logserver and
# reads from the logs until deadline
def read_logs_until(logsock, deadline):
    while True:
        logsock.settimeout(deadline - time.time())
        try:
            length = logsock.recv(4)
            length, = struct.unpack('>I', length)
            logsock.settimeout(deadline - time.time())
            record = ''
            while len(record) < length:
                record += logsock.recv(length - len(record))
            assert (len(record) == length)
        except socket.timeout:
            break
        except struct.error:
            continue
        sid, log_type, timestamp, rest = logger.log.deserialize_part(record)
        try:
            log = logger.type_to_obj[log_type].deserialize(sid, timestamp, rest)
        except KeyError as e:
            print e
            continue
        yield log

    logsock.settimeout(None)


# reads logs until interrupts
# no deadline
def read_logs(logsock):
    logsock.settimeout(None)
    while True:
        try:
            length = logsock.recv(4)
            length, = struct.unpack('>I', length)
            record = ''
            while len(record) < length:
                record += logsock.recv(length - len(record))
            assert (len(record) == length)
        except socket.timeout:
            break
        except struct.error:
            continue
        sid, log_type, timestamp, rest = logger.log.deserialize_part(record)
        log = logger.type_to_obj[log_type].deserialize(sid, timestamp, rest)
        yield log


# connector object that handles connection to the control
# hook for coinscope. Incudes functions to register transactions
# inv messages and receivng the current coinscope connections.
# Also functions to send send messages to peers.
class ConnectorSocket(object):
    # sockpath needs to reflect the location of coinscope
    def __init__(self, sockpath):
        self.sock = socket.socket(socket.AF_UNIX, socket.SOCK_STREAM, 0)
        self.sock.connect(sockpath)

    def send(self, msg):
        written = 0
        while written < len(msg):
            rv = self.sock.send(msg[written:], 0)
            if rv > 0:
                written = written + rv
            if rv < 0:
                raise Exception("Error on write (this happens automatically in python?)")

    def recv(self, length):
        msg = ''
        while len(msg) < length:
            msg += self.sock.recv(length - len(msg), socket.MSG_WAITALL)
        assert len(msg) == length
        return msg

    def register_tx(self, tx):
        assert isinstance(tx, TX) or isinstance(tx, CTransaction)

        if isinstance(tx, TX):
            tx = CTransaction.deserialize((tx.serialize(rtype=bin)))
        m = msg_tx()
        m.tx = tx
        cmsg = bitcoin_msg(m.serialize())
        ser = cmsg.serialize()
        self.send(ser)
        rid = self.recv(4)
        rid, = unpack('>I', rid)  # message is now saved and can be sent to users with this id
        return rid

    def register_verack(self):
        m = msg_verack()
        cmsg = bitcoin_msg(m.serialize())
        ser = cmsg.serialize()
        self.send(ser)
        rid = self.recv(4)
        rid, = unpack('>I', rid)
        return rid

    def register_getdata(self, hashes):
        m = msg_getdata()
        for h in hashes:
            assert len(h) == 32
            inv = CInv()
            inv.type = 1  # TX
            inv.hash = h
            m.inv.append(inv)
        cmsg = bitcoin_msg(m.serialize())
        ser = cmsg.serialize()
        self.send(ser)
        rid = self.recv(4)
        rid, = unpack('>I', rid)  # message is now saved and can be sent to users with this id
        return rid

    def register_getdata_msg(self, msg):
        cmsg = bitcoin_msg(msg.serialize())
        ser = cmsg.serialize()
        self.send(ser)
        rid = self.recv(4)
        rid, = unpack('>I', rid)
        return rid

    def register_inv(self, hashes):
        m = msg_inv()
        for h in hashes:
            assert len(h) == 32
            inv = CInv()
            inv.type = 1  # TX
            inv.hash = h
            m.inv.append(inv)
        cmsg = bitcoin_msg(m.serialize())
        ser = cmsg.serialize()
        self.send(ser)
        rid = self.recv(4)
        rid, = unpack('>I', rid)  # message is now saved and can be sent to users with this id
        return rid

    def register_cinv(self, m):
        cmsg = bitcoin_msg(m.serialize())
        ser = cmsg.serialize()
        self.send(ser)
        rid = self.recv(4)
        rid, = unpack('>I', rid)  # message is now saved and can be sent to users with this id
        return rid

    # Not useful to coinscpoe functionality, artifact of other work
    def register_dinv(self, hashes):
        m = msg_inv()
        m.command = 'd_inv'
        for h in hashes:
            assert len(h) == 32
            inv = CInv()
            inv.type = 1  # TX
            inv.hash = h
            m.inv.append(inv)
        cmsg = bitcoin_msg(m.serialize())
        ser = cmsg.serialize()
        self.send(ser)
        rid = self.recv(4)
        rid, = unpack('>I', rid)  # message is now saved and can be sent to users with this id
        return rid

    def register_ver(self, msg):
        cmsg = bitcoin_msg(msg.serialize())
        ser = cmsg.serialize()
        self.send(ser)
        rid = self.recv(4)
        rid, = unpack('>I', rid)
        return rid

    def broadcast(self, rid):
        cmsg = command_msg(commands.COMMAND_SEND_MSG, rid, (targets.BROADCAST,))
        ser = cmsg.serialize()
        self.send(ser)

    def send_to_nodes(self, rid, nodes):
        cmsg = command_msg(commands.COMMAND_SEND_MSG, rid, nodes)
        ser = cmsg.serialize()
        self.send(ser)

    def send_to_node(seld, rid, node):
        seld.send_to_nodes(rid, [node])

    # mapping that is only (ip, handle_id)
    def get_cxns(self):
        cmsg = command_msg(commands.COMMAND_GET_CXN, 0)
        ser = cmsg.serialize()
        self.send(ser)

        length = self.recv(4)
        length, = unpack('>I', length)
        infos = self.recv(length)
        # Each info chunk should be 36 bytes

        cur = 0
        while len(infos[cur:cur + 36]) > 0:
            cinfo = connection_info.deserialize(infos[cur:cur + 36])
            # print "{0} {1}:{2} - {3}:{4}".format(cinfo.handle_id, cinfo.remote_addr, cinfo.remote_port,
            #                                     cinfo.local_addr, cinfo.local_port)
            yield (cinfo.remote_addr, cinfo.handle_id[0])
            cur = cur + 36

    # Returns tuples of the form (ip:port, handle_id)
    # Used to create map that comes in handy when parsing logs
    def get_cxn(self):
        cmsg = command_msg(commands.COMMAND_GET_CXN, 0)
        ser = cmsg.serialize()
        self.send(ser)

        length = self.recv(4)
        length, = unpack('>I', length)
        infos = self.recv(length)
        # Each info chunk should be 36 bytes

        cur = 0
        while len(infos[cur:cur + 36]) > 0:
            cinfo = connection_info.deserialize(infos[cur:cur + 36])
            # print "{0} {1}:{2} - {3}:{4}".format(cinfo.handle_id, cinfo.remote_addr, cinfo.remote_port,
            #                                    cinfo.local_addr, cinfo.local_port)
            yield (cinfo.remote_addr + ':' + str(cinfo.remote_port), cinfo.handle_id[0])
            cur = cur + 36

    def connect_to_ip(self, ip, port, localip, localport):
        cmsg = connect_msg(ip, port, localip, localport)
        ser = cmsg.serialize()
        self.send(ser)

    def disconnect(self, handle_id):
        cmsg = command_msg(commands.COMMAND_DISCONNECT, 0, (handle_id,))
        ser = cmsg.serialize()
        self.send(ser)
