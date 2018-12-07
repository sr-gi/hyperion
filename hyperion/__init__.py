from ConfigParser import ConfigParser
from lib.conntools import *
import bitcoin

HOST = 'localhost'
PORT = 55357  # ASCII for data chart

MAINNET_PORT = 8333
TESTNET_PORT = 18333

# Loads configuration file
cfg = ConfigParser()
cfg.readfp(open('../coinscope.cfg'))

# Set paths for coinscope
COINSCOPE_PATH = cfg.get('coinscope', 'root_path')

# Set coinscope connectors
bitcoin.SelectParams(cfg.get('coinscope', 'network'))
CONNECTOR = ConnectorSocket(COINSCOPE_PATH + cfg.get('coinscope', 'conn_sock'))
