# Loads configuration file
from ConfigParser import ConfigParser

cfg = ConfigParser()
cfg.readfp(open('apps.cfg'))

# General variables
COINSCOPED_API_ADDR = cfg.get('general', 'coinscoped_api_addr')
COINSCOPED_API_PORT = int(cfg.get('general', 'coinscoped_api_port'))

# Nethealth
MAINNET_PORT = 8333
TESTNET_PORT = 18333
HALF_TARGET = 0.5
TARGET = float(cfg.get('nethealth', 'target'))
WIPE_TIME = int(cfg.get('nethealth', 'wipe_time'))
WIPE_DELAY = int(cfg.get('nethealth', 'wipe_delay'))

CARBON_SERVER = cfg.get('nethealth', 'carbon_server')
CARBON_PICKLE_PORT = int(cfg.get('nethealth', 'carbon_pickle_port'))
UPDATE_DELAY = int(cfg.get('nethealth', 'update_delay'))
