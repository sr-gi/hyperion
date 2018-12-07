from lib.common import unix2str
from lib.logger import log_types, update_types
from multiprocessing.connection import Client
from StringIO import StringIO
from bitcoin.messages import MsgSerializable
from getopt import getopt
from sys import argv
from apps import COINSCOPED_API_ADDR, COINSCOPED_API_PORT


commands = ['getpeer', 'getpeers', 'getpeersinfo', 'getpeerhistory', 'getpeercount', 'getlogs', 'getnetlogs', 'help']


def show_usage():
    print "usage: python coinscope-cli.py argument [additional_arguments]." \
          "\nArguments:" \
          "\ngetpeer <peer_ip:peer_port>: \treturns metadata abut the requested peer." \
          "\ngetpeers: \treturns the list of all connected peers with their corresponding coinscope id." \
          "\ngetpeersinfo: \treturns a list of all connected peers including all known metadata about them." \
          "\ngetpeercount: \treturns the total number of peers." \
          "\ngetlogs: \treturns a live parsing of the Bitcoin messages logs." \
          "\ngetnetlogs: \treturns a live parsing of the network logs." \
          "\nhelp: \t\tshows this message."


def deserialize_log(log, sock_type):
    if sock_type == 'bitcoin_msg_log':
        output = "[{0}] {1}: handle_id: {2}, is_sender: {3}, bitcoin_msg: {4}".format(
            unix2str(log.timestamp), log_types.str_mapping[log.log_type], log.handle_id,
            log.is_sender, MsgSerializable.stream_deserialize(StringIO(response.bitcoin_msg)))
    elif sock_type == 'bitcoin_log':
        output = "[{0}] {1}: handle: {2} update_type: {3}, remote: {4}:{5}, local: {6}:{7}, text: {8}".format(
            unix2str(log.timestamp), log_types.str_mapping[log.log_type], log.handle_id,
            update_types.str_mapping[log.update_type], log.remote_addr, log.remote_port, log.local_addr,
            log.local_port, log.text)

    else:
        raise Exception("Unknown socket type.")

    return output


if __name__ == '__main__':
    opts, args = getopt(argv[1:], '', commands)

    # Get args
    if len(args) > 0:
        command = args[0]
    else:
        raise Exception("Argument missing. Use help for usage information.")

    if command in ['getpeer', 'getpeerhistory']:
        if len(args) == 2:
            arg = args[1]
        elif len(args) == 3:
            command = 'getpeerold'
            arg = args[1:]
        else:
            raise Exception("Argument missing. Use help for usage information.")
    else:
        arg = None

    conn = Client((COINSCOPED_API_ADDR, COINSCOPED_API_PORT))

    if command in ['help']:
        show_usage()

    elif command in ['getpeer', 'getpeers', 'getpeersinfo', 'getpeercount', 'getpeerold', 'getpeerhistory']:

        if arg:
            conn.send((command, arg))
        else:
            conn.send(command)

        response = conn.recv()

        if command in ['getpeer', 'getpeers', 'getpeercount', 'getpeerold', 'getpeerhistory']:
            output = response
        elif command == 'getpeersinfo':
            output = '{\n'
            output += ''.join('\t%s:%s\n' % (peer_ip, meta) for peer_ip, meta in response.iteritems())
            output += '}'

        print output
        conn.close()

    elif command in ['getlogs', 'getnetlogs']:
        conn.send(command)

        while True:
            response = conn.recv()

            if command == 'getlogs':
                try:
                    print deserialize_log(response, 'bitcoin_msg_log')
                except ValueError:
                    continue
            elif command == 'getnetlogs':
                print deserialize_log(response, 'bitcoin_log')

            elif command == 'getlogerrors':
                try:
                    deserialize_log(response, 'bitcoin_msg_log')
                except ValueError:
                    print "[{0}] {1}: handle_id: {2}, is_sender: {3}, error: wrong magic number".format(
                        unix2str(response.timestamp), log_types.str_mapping[response.log_type], response.handle_id,
                        response.is_sender)

    else:
        raise Exception("Invalid command. Use help for usage information.")

