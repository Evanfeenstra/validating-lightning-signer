use core::fmt;
use std::collections::HashMap;
use std::fmt::Debug;
use std::sync::{Arc, Mutex, MutexGuard};

use bitcoin;
use bitcoin::util::bip32::ExtendedPrivKey;
use bitcoin::{Network, OutPoint, Script, SigHashType};
use bitcoin_hashes::core::fmt::{Error, Formatter};
use bitcoin_hashes::sha256d::Hash as Sha256dHash;
use bitcoin_hashes::Hash;
use lightning::chain::keysinterface::{ChannelKeys, KeysInterface};
use lightning::ln::chan_utils::{ChannelPublicKeys, HTLCOutputInCommitment, TxCreationKeys};
use lightning::ln::msgs::UnsignedChannelAnnouncement;
use lightning::util::logger::Logger;
use secp256k1::{All, PublicKey, Secp256k1, SecretKey, Signature};
use tonic::Status;

use crate::server::my_keys_manager::{MyKeysManager, INITIAL_COMMITMENT_NUMBER};
use crate::tx::tx::{
    build_commitment_tx, get_commitment_transaction_number_obscure_factor, CommitmentInfo2,
    HTLCInfo,
};
use crate::util::crypto_utils::{
    derive_public_key, derive_public_revocation_key, payload_for_p2wpkh,
};
use crate::util::enforcing_trait_impls::EnforcingChannelKeys;
use crate::util::invoice_utils;

use backtrace::Backtrace;
use std::time::{SystemTime, UNIX_EPOCH};

#[derive(PartialEq, Eq, Hash, Clone, Copy)]
pub struct ChannelId(pub [u8; 32]);
// NOTE - this "ChannelId" does *not* correspond to the "channel_id"
// defined in BOLT #2.

impl Debug for ChannelId {
    fn fmt(&self, f: &mut Formatter<'_>) -> Result<(), Error> {
        f.write_str(hex::encode(self.0).as_str())
    }
}

impl fmt::Display for ChannelId {
    // BEGIN NOT TESTED
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        f.write_str(hex::encode(self.0).as_str())
    }
    // END NOT TESTED
}

pub struct RemoteChannelConfig {
    pub to_self_delay: u16,
    pub shutdown_script: Script,
    pub funding_outpoint: OutPoint,
}

pub struct Channel {
    pub node: Arc<Node>,
    pub logger: Arc<Logger>,
    pub keys: EnforcingChannelKeys,
    pub secp_ctx: Secp256k1<All>,
    pub channel_value_satoshi: u64,
    pub local_to_self_delay: u16,
    pub is_outbound: bool,
    pub remote_config: Option<RemoteChannelConfig>,
}

impl Debug for Channel {
    // BEGIN NOT TESTED
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        f.write_str("channel")
    }
    // END NOT TESTED
}

impl Channel {
    // BEGIN NOT TESTED
    pub(super) fn invalid_argument(&self, msg: impl Into<String>) -> Status {
        let s = msg.into();
        log_error!(self, "INVALID ARGUMENT: {}", &s);
        log_error!(self, "BACKTRACE:\n{:?}", Backtrace::new());
        Status::invalid_argument(s)
    }
    // END NOT TESTED

    // BEGIN NOT TESTED
    pub(super) fn internal_error(&self, msg: impl Into<String>) -> Status {
        let s = msg.into();
        log_error!(self, "INTERNAL ERROR: {}", &s);
        log_error!(self, "BACKTRACE:\n{:?}", Backtrace::new());
        Status::internal(s)
    }
    // END NOT TESTED

    // Phase 2
    fn make_remote_tx_keys(
        &self,
        per_commitment_point: &PublicKey,
    ) -> Result<TxCreationKeys, Status> {
        let keys = &self.keys.inner;
        let local_pubkeys = keys.pubkeys();

        #[rustfmt::skip]
        let remote_pubkeys = keys.remote_pubkeys().as_ref()
            .ok_or_else(|| self.invalid_argument("channel must be accepted"))?;

        Ok(self.make_tx_keys(per_commitment_point, remote_pubkeys, local_pubkeys))
    }

    pub(crate) fn make_local_tx_keys(
        &self,
        per_commitment_point: &PublicKey,
    ) -> Result<TxCreationKeys, Status> {
        let keys = &self.keys.inner;
        let local_pubkeys = keys.pubkeys();

        #[rustfmt::skip]
        let remote_pubkeys = keys.remote_pubkeys().as_ref()
            .ok_or_else(|| self.invalid_argument("channel must be accepted"))?;

        Ok(self.make_tx_keys(per_commitment_point, local_pubkeys, remote_pubkeys))
    }

    fn make_tx_keys(
        &self,
        per_commitment_point: &PublicKey,
        a_pubkeys: &ChannelPublicKeys,
        b_pubkeys: &ChannelPublicKeys,
    ) -> TxCreationKeys {
        TxCreationKeys::new(
            &self.secp_ctx,
            &per_commitment_point,
            &a_pubkeys.delayed_payment_basepoint,
            &a_pubkeys.htlc_basepoint,
            &b_pubkeys.revocation_basepoint,
            &b_pubkeys.payment_basepoint,
            &b_pubkeys.htlc_basepoint,
        )
        .expect("failed to derive keys")
    }

    // TODO phase 2
    pub fn sign_remote_commitment(
        &self,
        feerate_per_kw: u64,
        commitment_tx: &bitcoin::Transaction,
        per_commitment_point: &PublicKey,
        htlcs: &[&HTLCOutputInCommitment],
        to_self_delay: u16,
    ) -> Result<(Signature, Vec<Signature>), Status> {
        let tx_keys = self.make_remote_tx_keys(per_commitment_point)?;
        self.keys
            .sign_remote_commitment(
                feerate_per_kw,
                commitment_tx,
                &tx_keys,
                htlcs,
                to_self_delay,
                &self.secp_ctx,
            )
            .map_err(|_| self.internal_error("sign_remote_commitment failed"))
    }

    pub fn sign_channel_announcement(
        &self,
        msg: &UnsignedChannelAnnouncement,
    ) -> Result<Signature, ()> {
        self.keys.sign_channel_announcement(msg, &self.secp_ctx)
    }

    pub fn accept_remote_points(&mut self, channel_points: &ChannelPublicKeys) {
        self.keys.set_remote_channel_pubkeys(channel_points);
    }

    pub fn accept_remote_config(
        &mut self,
        remote_to_self_delay: u16,
        shutdown_script: Script,
        funding_outpoint: OutPoint,
    ) {
        self.remote_config = Some(RemoteChannelConfig {
            to_self_delay: remote_to_self_delay,
            shutdown_script,
            funding_outpoint,
        });
    }

    fn get_commitment_transaction_number_obscure_factor(&self) -> u64 {
        get_commitment_transaction_number_obscure_factor(
            &self.secp_ctx,
            self.keys.payment_base_key(),
            &self
                .keys
                .remote_pubkeys()
                .as_ref()
                .expect("channel must be accepted")
                .payment_basepoint,
            self.is_outbound,
        )
    }

    pub fn get_per_commitment_point(&self, commitment_number: u64) -> PublicKey {
        let seed = self.keys.commitment_seed();
        MyKeysManager::per_commitment_point(&self.secp_ctx, seed, commitment_number)
    }

    pub fn get_per_commitment_secret(&self, commitment_number: u64) -> SecretKey {
        let seed = self.keys.commitment_seed();
        MyKeysManager::per_commitment_secret(seed, commitment_number)
    }

    pub fn ready(
        &mut self,
        channel_points: &ChannelPublicKeys,
        remote_to_self_delay: u16,
        shutdown_script: Script,
        funding_outpoint: OutPoint,
    ) {
        self.accept_remote_points(channel_points);
        self.accept_remote_config(remote_to_self_delay, shutdown_script, funding_outpoint);
    }

    // BEGIN NOT TESTED
    pub fn is_ready(&self) -> bool {
        self.remote_config.is_some()
    }
    // END NOT TESTED

    pub fn build_commitment_tx(
        &self,
        remote_per_commitment_point: &PublicKey,
        commitment_number: u64,
        info: &CommitmentInfo2,
    ) -> Result<
        (
            bitcoin::Transaction,
            Vec<Script>,
            Vec<HTLCOutputInCommitment>,
        ),
        Status,
    > {
        let keys = self.make_remote_tx_keys(remote_per_commitment_point)?;
        let obscured_commitment_transaction_number = self
            .get_commitment_transaction_number_obscure_factor()
            ^ (INITIAL_COMMITMENT_NUMBER - commitment_number);
        let remote_config = self
            .remote_config
            .as_ref()
            .ok_or_else(|| self.invalid_argument("channel not accepted yet"))?;
        Ok(build_commitment_tx(
            &keys,
            info,
            obscured_commitment_transaction_number,
            remote_config.funding_outpoint,
        ))
    }

    pub fn build_remote_commitment_info(
        &self,
        remote_per_commitment_point: &PublicKey,
        to_local_value: u64,
        to_remote_value: u64,
        offered_htlcs: Vec<HTLCInfo>,
        received_htlcs: Vec<HTLCInfo>,
    ) -> Result<CommitmentInfo2, Status> {
        let local_pubkeys = self.keys.pubkeys();
        let remote_config = self
            .remote_config
            .as_ref()
            .ok_or_else(|| self.invalid_argument("channel not ready"))?;
        let remote_pubkeys = self
            .keys
            .remote_pubkeys()
            .as_ref()
            .ok_or_else(|| self.invalid_argument("channel not ready"))?;

        let secp_ctx = &self.secp_ctx;

        let to_local_delayed_key = derive_public_key(
            secp_ctx,
            &remote_per_commitment_point,
            &remote_pubkeys.delayed_payment_basepoint,
        )
        .map_err(|err| {
            // BEGIN NOT TESTED
            self.internal_error(format!("could not derive to_local_delayed_key: {}", err))
            // END NOT TESTED
        })?;
        let remote_key = derive_public_key(
            secp_ctx,
            &remote_per_commitment_point,
            &local_pubkeys.payment_basepoint,
        )
        .map_err(|err| self.internal_error(format!("could not derive remote_key: {}", err)))?;
        let revocation_key = derive_public_revocation_key(
            secp_ctx,
            &remote_per_commitment_point,
            &local_pubkeys.revocation_basepoint,
        )
        .map_err(|err| self.internal_error(format!("could not derive revocation key: {}", err)))?;
        let to_remote_address = payload_for_p2wpkh(&remote_key);
        Ok(CommitmentInfo2 {
            to_remote_address,
            to_remote_value,
            revocation_key,
            to_local_delayed_key,
            to_local_value,
            to_local_delay: remote_config.to_self_delay,
            offered_htlcs,
            received_htlcs,
        })
    }

    pub fn build_local_commitment_info(
        &self,
        per_commitment_point: &PublicKey,
        to_local_value: u64,
        to_remote_value: u64,
        offered_htlcs: Vec<HTLCInfo>,
        received_htlcs: Vec<HTLCInfo>,
    ) -> Result<CommitmentInfo2, Status> {
        let local_pubkeys = self.keys.pubkeys();
        let remote_pubkeys = self
            .keys
            .remote_pubkeys()
            .as_ref()
            .ok_or_else(|| self.invalid_argument("channel not ready"))?;
        let secp_ctx = &self.secp_ctx;

        let to_local_delayed_key = derive_public_key(
            secp_ctx,
            &per_commitment_point,
            &local_pubkeys.delayed_payment_basepoint,
        )
        .map_err(|err| {
            // BEGIN NOT TESTED
            self.internal_error(format!("could not derive to_local_delayed_key: {}", err))
            // END NOT TESTED
        })?;
        let remote_key = derive_public_key(
            secp_ctx,
            &per_commitment_point,
            &remote_pubkeys.payment_basepoint,
        )
        .map_err(|err| self.internal_error(format!("could not derive remote_key: {}", err)))?;
        let revocation_key = derive_public_revocation_key(
            secp_ctx,
            &per_commitment_point,
            &remote_pubkeys.revocation_basepoint,
        )
        .map_err(|err| self.internal_error(format!("could not derive revocation_key: {}", err)))?;
        let to_remote_address = payload_for_p2wpkh(&remote_key);
        Ok(CommitmentInfo2 {
            to_remote_address,
            to_remote_value,
            revocation_key,
            to_local_delayed_key,
            to_local_value,
            to_local_delay: self.local_to_self_delay,
            offered_htlcs,
            received_htlcs,
        })
    }

    pub fn sign_remote_commitment_tx_phase2(
        &self,
        remote_per_commitment_point: &PublicKey,
        commitment_number: u64,
        feerate_per_kw: u64,
        to_local_value: u64,
        to_remote_value: u64,
        offered_htlcs: Vec<HTLCInfo>,
        received_htlcs: Vec<HTLCInfo>,
    ) -> Result<(Vec<u8>, Vec<Vec<u8>>), Status> {
        let info = self.build_remote_commitment_info(
            remote_per_commitment_point,
            to_local_value,
            to_remote_value,
            offered_htlcs.clone(),
            received_htlcs.clone(),
        )?;

        let (tx, _scripts, htlcs) =
            self.build_commitment_tx(remote_per_commitment_point, commitment_number, &info)?;
        let keys = self.make_remote_tx_keys(remote_per_commitment_point)?;

        let mut htlc_refs = Vec::new();
        for htlc in htlcs.iter() {
            htlc_refs.push(htlc); // NOT TESTED
        }
        let remote_config = self
            .remote_config
            .as_ref()
            .ok_or_else(|| self.invalid_argument("channel not accepted yet"))?;
        let sigs = self
            .keys
            .sign_remote_commitment(
                feerate_per_kw,
                &tx,
                &keys,
                htlc_refs.as_slice(),
                remote_config.to_self_delay,
                &self.secp_ctx,
            )
            .map_err(|_| self.internal_error("failed to sign"))?;
        let mut sig = sigs.0.serialize_der().to_vec();
        sig.push(SigHashType::All as u8);
        let mut htlc_sigs = Vec::new();
        for htlc_signature in sigs.1 {
            // BEGIN NOT TESTED
            let mut htlc_sig = htlc_signature.serialize_der().to_vec();
            htlc_sig.push(SigHashType::All as u8);
            htlc_sigs.push(htlc_sig);
            // END NOT TESTED
        }
        Ok((sig, htlc_sigs))
    }
}

pub struct Node {
    pub logger: Arc<Logger>,
    pub(crate) keys_manager: MyKeysManager,
    channels: Mutex<HashMap<ChannelId, Channel>>,
}

impl Node {
    pub fn new(logger: &Arc<Logger>, seed: &[u8; 32], network: Network) -> Node {
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("Time went backwards");

        Node {
            logger: Arc::clone(logger),
            keys_manager: MyKeysManager::new(
                seed,
                network,
                Arc::clone(logger),
                now.as_secs(),
                now.subsec_nanos(),
            ),
            channels: Mutex::new(HashMap::new()),
        }
    }

    #[allow(dead_code)]
    // BEGIN NOT TESTED
    pub(super) fn invalid_argument(&self, msg: impl Into<String>) -> Status {
        let s = msg.into();
        log_error!(self, "INVALID ARGUMENT: {}", &s);
        log_error!(self, "BACKTRACE:\n{:?}", Backtrace::new());
        Status::invalid_argument(s)
    }
    // END NOT TESTED

    // BEGIN NOT TESTED
    pub(super) fn internal_error(&self, msg: impl Into<String>) -> Status {
        let s = msg.into();
        log_error!(self, "INTERNAL ERROR: {}", &s);
        log_error!(self, "BACKTRACE:\n{:?}", Backtrace::new());
        Status::internal(s)
    }
    // END NOT TESTED

    /// TODO leaking secret
    pub fn get_node_secret(&self) -> SecretKey {
        self.keys_manager.get_node_secret()
    }

    /// TODO leaking secret
    pub fn get_onion_rand(&self) -> (SecretKey, [u8; 32]) {
        self.keys_manager.get_onion_rand()
    }

    /// Get destination redeemScript to encumber static protocol exit points.
    pub fn get_destination_script(&self) -> Script {
        self.keys_manager.get_destination_script()
    }

    /// Get shutdown_pubkey to use as PublicKey at channel closure
    pub fn get_shutdown_pubkey(&self) -> PublicKey {
        self.keys_manager.get_shutdown_pubkey()
    }

    /// Get a unique temporary channel id. Channels will be referred
    /// to by this until the funding transaction is created, at which
    /// point they will use the outpoint in the funding transaction.
    pub fn get_channel_id(&self) -> [u8; 32] {
        self.keys_manager.get_channel_id()
    }

    pub fn get_bip32_key(&self) -> &ExtendedPrivKey {
        self.keys_manager.get_bip32_key()
    }

    pub fn sign_node_announcement(&self, na: &Vec<u8>) -> Result<Vec<u8>, Status> {
        let secp_ctx = Secp256k1::signing_only();
        let na_hash = Sha256dHash::hash(na);
        let encmsg = ::secp256k1::Message::from_slice(&na_hash[..])
            .map_err(|err| self.internal_error(format!("encmsg failed: {}", err)))?;
        let sig = secp_ctx.sign(&encmsg, &self.get_node_secret());
        let res = sig.serialize_der().to_vec();
        Ok(res)
    }

    pub fn sign_channel_update(&self, cu: &Vec<u8>) -> Result<Vec<u8>, Status> {
        let secp_ctx = Secp256k1::signing_only();
        let cu_hash = Sha256dHash::hash(cu);
        let encmsg = ::secp256k1::Message::from_slice(&cu_hash[..])
            .map_err(|err| self.internal_error(format!("encmsg failed: {}", err)))?;
        let sig = secp_ctx.sign(&encmsg, &self.get_node_secret());
        let res = sig.serialize_der().to_vec();
        Ok(res)
    }

    pub fn sign_invoice(
        &self,
        data_part: &Vec<u8>,
        human_readable_part: &String,
    ) -> Result<Vec<u8>, Status> {
        use bech32::CheckBase32;

        let hash = invoice_utils::hash_from_parts(
            human_readable_part.as_bytes(),
            &data_part.check_base32().expect("needs to be base32 data"),
        );

        let secp_ctx = Secp256k1::signing_only();
        let encmsg = ::secp256k1::Message::from_slice(&hash[..])
            .map_err(|err| self.internal_error(format!("encmsg failed: {}", err)))?;
        let sig = secp_ctx.sign_recoverable(&encmsg, &self.get_node_secret());
        let (rid, sig) = sig.serialize_compact();
        let mut res = sig.to_vec();
        res.push(rid.to_i32() as u8);
        Ok(res)
    }

    pub fn sign_message(&self, message: &Vec<u8>) -> Result<Vec<u8>, Status> {
        let mut buffer = String::from("Lightning Signed Message:").into_bytes();
        buffer.extend(message);
        let secp_ctx = Secp256k1::signing_only();
        let hash = Sha256dHash::hash(&buffer);
        let encmsg = ::secp256k1::Message::from_slice(&hash[..])
            .map_err(|err| self.internal_error(format!("encmsg failed: {}", err)))?;
        let sig = secp_ctx.sign_recoverable(&encmsg, &self.get_node_secret());
        let (rid, sig) = sig.serialize_compact();
        let mut res = sig.to_vec();
        res.push(rid.to_i32() as u8);
        Ok(res)
    }

    pub fn channels(&self) -> MutexGuard<HashMap<ChannelId, Channel>> {
        self.channels.lock().unwrap()
    }
}

impl Debug for Node {
    // BEGIN NOT TESTED
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        f.write_str("node")
    }
    // END NOT TESTED
}