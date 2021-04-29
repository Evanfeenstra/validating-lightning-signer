use bitcoin::util::address::Payload;
use lightning::ln::chan_utils::ChannelPublicKeys;

use crate::fmt;

// Debug printer for ChannelPublicKeys which doesn't have one.
// BEGIN NOT TESTED
pub struct DebugChannelPublicKeys<'a>(pub &'a ChannelPublicKeys);
impl<'a> fmt::Debug for DebugChannelPublicKeys<'a> {
    fn fmt(&self, f: &mut fmt::Formatter) -> Result<(), fmt::Error> {
        f.debug_struct("ChannelPublicKeys")
            .field("funding_pubkey", &self.0.funding_pubkey)
            .field("revocation_basepoint", &self.0.revocation_basepoint)
            .field("payment_point", &self.0.payment_point)
            .field(
                "delayed_payment_basepoint",
                &self.0.delayed_payment_basepoint,
            )
            .field("htlc_basepoint", &self.0.htlc_basepoint)
            .finish()
    }
}
// END NOT TESTED
macro_rules! log_channel_public_keys {
    ($obj: expr) => {
        &crate::util::debug_utils::DebugChannelPublicKeys(&$obj)
    };
}

// Debug printer for Payload which uses hex encoded strings.
// BEGIN NOT TESTED
pub struct DebugPayload<'a>(pub &'a Payload);
impl<'a> fmt::Debug for DebugPayload<'a> {
    fn fmt(&self, f: &mut fmt::Formatter) -> Result<(), fmt::Error> {
        match *self.0 {
            Payload::PubkeyHash(ref hash) => write!(f, "{}", hex::encode(&hash)),
            Payload::ScriptHash(ref hash) => write!(f, "{}", hex::encode(&hash)),
            Payload::WitnessProgram {
                version: ver,
                program: ref prog,
            } => f
                .debug_struct("WitnessProgram")
                .field("version", &ver.to_u8())
                .field("program", &hex::encode(&prog))
                .finish(),
        }
    }
}
// END NOT TESTED