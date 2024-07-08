use crate::TxId;

#[derive(Copy, Clone, PartialEq, Eq, Hash)]
pub enum NetworkMessage {
    INV(TxId),
    GETDATA(TxId),
    TX(TxId),
}

impl NetworkMessage {
    pub fn inner(&self) -> &TxId {
        match self {
            NetworkMessage::INV(x) => x,
            NetworkMessage::GETDATA(x) => x,
            NetworkMessage::TX(x) => x,
        }
    }

    fn get_size(&self) -> u32 {
        // FIXME: Add some sizes that make sense
        match self {
            NetworkMessage::INV(_) => 32,
            NetworkMessage::GETDATA(_) => 32,
            NetworkMessage::TX(_) => 150,
        }
    }

    pub fn is_inv(&self) -> bool {
        matches!(self, NetworkMessage::INV(..))
    }

    pub fn is_get_data(&self) -> bool {
        matches!(self, NetworkMessage::GETDATA(..))
    }

    pub fn is_tx(&self) -> bool {
        matches!(self, NetworkMessage::TX(..))
    }
}

impl std::fmt::Display for NetworkMessage {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let (m, txid) = match self {
            NetworkMessage::INV(x) => ("inv", x),
            NetworkMessage::GETDATA(x) => ("getdata", x),
            NetworkMessage::TX(x) => ("tx", x),
        };

        write!(f, "{m}(txid: {txid:x})")
    }
}
