# Loads configuration file
from ConfigParser import ConfigParser

cfg = ConfigParser()
cfg.readfp(open('apps.cfg'))

# General variables
COINSCOPED_API_ADDR = cfg.get('general', 'coinscoped_api_addr')
COINSCOPED_API_PORT = cfg.get('general', 'coinscoped_api_port')

# Nethealth
MAINNET_PORT = 8333
TESTNET_PORT = 18333
HALF_TARGET = 0.5
TARGET = cfg.get('nethealth', 'target')
WIPE_TIME = cfg.get('nethealth', 'wipe_time')
WIPE_DELAY = cfg.get('nethealth', 'wipe_delay')

CARBON_SERVER = cfg.get('nethealth', 'carbon_server')
CARBON_PICKLE_PORT = cfg.get('nethealth', 'carbon_pickle_port')
UPDATE_DELAY = cfg.get('nethealth', 'update_delay')
