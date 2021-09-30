#[cfg(test)]
mod tests {
    use bitcoin::{self, OutPoint, Script, Transaction, TxIn, TxOut, Txid};
    use lightning::ln::chan_utils::get_revokeable_redeemscript;
    use test_env_log::test;

    use crate::channel::{Channel, ChannelBase};
    use crate::node::SpendType::{P2shP2wpkh, P2wpkh};
    use crate::util::crypto_utils::signature_to_bitcoin_vec;
    use crate::util::status::{Code, Status};
    use crate::util::test_utils::*;

    fn make_test_delayed_sweep_tx(
        txid: Txid,
        vout: u32,
        contest_delay: u16,
        script_pubkey: Script,
        amount_sat: u64,
    ) -> Transaction {
        Transaction {
            version: 2,
            lock_time: 0,
            input: vec![TxIn {
                previous_output: OutPoint { txid, vout },
                script_sig: Script::new(),
                sequence: contest_delay as u32,
                witness: vec![],
            }],
            output: vec![TxOut {
                script_pubkey: script_pubkey,
                value: amount_sat,
            }],
        }
    }

    const HOLD_COMMIT_NUM: u64 = 53;

    fn sign_delayed_sweep_with_mutators<MakeDestination, InputMutator>(
        make_dest: MakeDestination,
        mutate_signing_input: InputMutator,
    ) -> Result<(), Status>
    where
        MakeDestination: Fn(&TestNodeContext) -> (Script, Vec<u32>),
        InputMutator:
            Fn(&mut Channel, &mut Transaction, &mut usize, &mut u64, &mut Script, &mut u64),
    {
        let next_holder_commit_num = HOLD_COMMIT_NUM;
        let next_counterparty_commit_num = HOLD_COMMIT_NUM + 1;
        let next_counterparty_revoke_num = next_counterparty_commit_num - 1;
        let (node_ctx, chan_ctx) = setup_funded_channel(
            next_holder_commit_num,
            next_counterparty_commit_num,
            next_counterparty_revoke_num,
        );
        let commit_tx_ctx = setup_validated_holder_commitment(
            &node_ctx,
            &chan_ctx,
            HOLD_COMMIT_NUM,
            |_commit_tx_ctx| {},
            |_keys| {},
        )?;

        let (sig, tx, per_commitment_point, input, redeemscript, amount_sat) = node_ctx
            .node
            .with_ready_channel(&chan_ctx.channel_id, |chan| {
                let mut input = 0;
                let built_commit = commit_tx_ctx
                    .tx
                    .as_ref()
                    .unwrap()
                    .trust()
                    .built_transaction()
                    .clone();
                let built_commit_tx = &built_commit.transaction;
                let built_commit_txid = &built_commit.txid;

                let mut commit_num = HOLD_COMMIT_NUM;
                let per_commitment_point = chan.get_per_commitment_point(commit_num)?;
                let keys = chan.make_holder_tx_keys(&per_commitment_point).unwrap();

                let contest_delay = chan.setup.counterparty_selected_contest_delay;
                let mut redeemscript = get_revokeable_redeemscript(
                    &keys.revocation_key,
                    contest_delay,
                    &keys.broadcaster_delayed_payment_key,
                );
                let to_local_outndx = 4;
                let mut amount_sat = built_commit_tx.output[to_local_outndx].value;
                assert_eq!(amount_sat, 1979997);

                let (script_pubkey, wallet_path) = make_dest(&node_ctx);

                let fee = 1_000;
                let mut tx = make_test_delayed_sweep_tx(
                    built_commit_txid.clone(),
                    to_local_outndx as u32,
                    contest_delay,
                    script_pubkey,
                    amount_sat - fee,
                );

                mutate_signing_input(
                    chan,
                    &mut tx,
                    &mut input,
                    &mut commit_num,
                    &mut redeemscript,
                    &mut amount_sat,
                );

                let sig = chan.sign_delayed_sweep(
                    &tx,
                    input,
                    commit_num,
                    &redeemscript,
                    amount_sat,
                    &wallet_path,
                )?;
                Ok((
                    sig,
                    tx,
                    per_commitment_point,
                    input,
                    redeemscript,
                    amount_sat,
                ))
            })?;

        let delayed_pubkey = get_channel_delayed_payment_pubkey(
            &node_ctx.node,
            &chan_ctx.channel_id,
            &per_commitment_point,
        );

        check_signature(
            &tx,
            input,
            signature_to_bitcoin_vec(sig),
            &delayed_pubkey,
            amount_sat,
            &redeemscript,
        );

        Ok(())
    }

    // policy-delayed-sweep-destination-allowlisted
    #[test]
    fn sign_delayed_to_local_wallet_p2wpkh_success() {
        assert_status_ok!(sign_delayed_sweep_with_mutators(
            |node_ctx| { make_test_wallet_dest(node_ctx, 19, P2wpkh) },
            |_chan, _tx, _input, _commit_num, _redeemscript, _amount_sat| {},
        ));
    }

    // policy-delayed-sweep-destination-allowlisted
    #[test]
    fn sign_delayed_to_local_wallet_p2shwpkh_success() {
        assert_status_ok!(sign_delayed_sweep_with_mutators(
            |node_ctx| { make_test_wallet_dest(node_ctx, 19, P2shP2wpkh) },
            |_chan, _tx, _input, _commit_num, _redeemscript, _amount_sat| {},
        ));
    }

    // policy-delayed-sweep-destination-allowlisted
    #[test]
    fn sign_delayed_to_local_allowlist_p2wpkh_success() {
        assert_status_ok!(sign_delayed_sweep_with_mutators(
            |node_ctx| { make_test_nonwallet_dest(node_ctx, 3, P2wpkh) },
            |chan, _tx, _input, _commit_num, _redeemscript, _amount_sat| {
                chan.node
                    .upgrade()
                    .unwrap()
                    .add_allowlist(&vec![
                        "tb1qg975h6gdx5mryeac72h6lj2nzygugxhyk6dnhr".to_string()
                    ])
                    .expect("add_allowlist");
            },
        ));
    }

    // policy-delayed-sweep-destination-allowlisted
    #[test]
    fn sign_delayed_to_local_allowlist_p2shwpkh_success() {
        assert_status_ok!(sign_delayed_sweep_with_mutators(
            |node_ctx| { make_test_nonwallet_dest(node_ctx, 3, P2shP2wpkh) },
            |chan, _tx, _input, _commit_num, _redeemscript, _amount_sat| {
                chan.node
                    .upgrade()
                    .unwrap()
                    .add_allowlist(&vec!["2MspRgcQvaVN2RkpumN1X8GkzsE7BVTTb6y".to_string()])
                    .expect("add_allowlist");
            },
        ));
    }

    // policy-delayed-sweep-destination-allowlisted
    #[test]
    fn sign_delayed_to_local_with_unknown_dest() {
        assert_failed_precondition_err!(
            sign_delayed_sweep_with_mutators(
                |node_ctx| { make_test_nonwallet_dest(node_ctx, 3, P2shP2wpkh) },
                |_chan, _tx, _input, _commit_num, _redeemscript, _amount_sat| {},
            ),
            "policy failure: validate_delayed_sweep: destination is not in wallet or allowlist"
        );
    }

    // policy-delayed-sweep-destination-allowlisted
    #[test]
    fn sign_delayed_sweep_with_wrong_wallet_path() {
        assert_failed_precondition_err!(
            sign_delayed_sweep_with_mutators(
                |node_ctx| {
                    // Build the dest from index 19, but report index 21.
                    (make_test_wallet_dest(node_ctx, 19, P2wpkh).0, vec![21])
                },
                |_chan, _tx, _input, _commit_num, _redeemscript, _amount_sat| {},
            ),
            "policy failure: validate_delayed_sweep: destination is not in wallet or allowlist"
        );
    }

    #[test]
    fn sign_delayed_sweep_with_bad_num_inputs() {
        assert_failed_precondition_err!(
            sign_delayed_sweep_with_mutators(
                |node_ctx| { make_test_wallet_dest(node_ctx, 19, P2wpkh) },
                |_chan, tx, _input, _commit_num, _redeemscript, _amount_sat| {
                    tx.input.push(tx.input[0].clone());
                },
            ),
            "transaction format: validate_delayed_sweep: \
             bad number of delayed sweep inputs: 2 != 1"
        );
    }

    #[test]
    fn sign_delayed_sweep_with_bad_num_outputs() {
        assert_failed_precondition_err!(
            sign_delayed_sweep_with_mutators(
                |node_ctx| { make_test_wallet_dest(node_ctx, 19, P2wpkh) },
                |_chan, tx, _input, _commit_num, _redeemscript, _amount_sat| {
                    tx.output.push(tx.output[0].clone());
                },
            ),
            "transaction format: validate_delayed_sweep: \
             bad number of delayed sweep outputs: 2 != 1"
        );
    }

    #[test]
    fn sign_delayed_sweep_with_bad_input_index() {
        assert_failed_precondition_err!(
            sign_delayed_sweep_with_mutators(
                |node_ctx| { make_test_wallet_dest(node_ctx, 19, P2wpkh) },
                |_chan, _tx, input, _commit_num, _redeemscript, _amount_sat| {
                    *input = 1;
                },
            ),
            "transaction format: validate_delayed_sweep: bad input index: 1 != 0"
        );
    }

    // policy-delayed-sweep-version
    #[test]
    fn sign_delayed_sweep_with_bad_version() {
        assert_failed_precondition_err!(
            sign_delayed_sweep_with_mutators(
                |node_ctx| { make_test_wallet_dest(node_ctx, 19, P2wpkh) },
                |_chan, tx, _input, _commit_num, _redeemscript, _amount_sat| {
                    tx.version = 3;
                },
            ),
            "transaction format: validate_delayed_sweep: bad delayed sweep version: 3"
        );
    }

    // policy-delayed-sweep-locktime
    #[test]
    fn sign_delayed_sweep_with_bad_locktime() {
        assert_failed_precondition_err!(
            sign_delayed_sweep_with_mutators(
                |node_ctx| { make_test_wallet_dest(node_ctx, 19, P2wpkh) },
                |_chan, tx, _input, _commit_num, _redeemscript, _amount_sat| {
                    tx.lock_time = 1_000_000;
                },
            ),
            "transaction format: validate_delayed_sweep: bad delayed sweep locktime: 1000000 > 0"
        );
    }

    // policy-delayed-sweep-sequence
    #[test]
    fn sign_delayed_sweep_with_bad_sequence() {
        assert_failed_precondition_err!(
            sign_delayed_sweep_with_mutators(
                |node_ctx| { make_test_wallet_dest(node_ctx, 19, P2wpkh) },
                |_chan, tx, _input, _commit_num, _redeemscript, _amount_sat| {
                    tx.input[0].sequence = 42;
                },
            ),
            "transaction format: validate_delayed_sweep: bad delayed sweep sequence: 42 != 7"
        );
    }

    #[test]
    fn sign_delayed_sweep_with_fee_underflow() {
        assert_failed_precondition_err!(
            sign_delayed_sweep_with_mutators(
                |node_ctx| { make_test_wallet_dest(node_ctx, 19, P2wpkh) },
                |_chan, _tx, _input, _commit_num, _redeemscript, amount_sat| {
                    *amount_sat -= 100_000;
                },
            ),
            "policy failure: delayed sweep fee underflow: 1879997 - 1978997"
        );
    }

    // policy-delayed-sweep-fee-range
    #[test]
    fn sign_delayed_sweep_with_fee_too_small() {
        assert_failed_precondition_err!(
            sign_delayed_sweep_with_mutators(
                |node_ctx| { make_test_wallet_dest(node_ctx, 19, P2wpkh) },
                |_chan, tx, _input, _commit_num, _redeemscript, amount_sat| {
                    *amount_sat = tx.output[0].value; // fee = 0
                },
            ),
            "policy failure: validate_fee: validate_delayed_sweep: fee below minimum: 0 < 100"
        );
    }

    // policy-delayed-sweep-fee-range
    #[test]
    fn sign_delayed_sweep_with_fee_too_large() {
        assert_failed_precondition_err!(
            sign_delayed_sweep_with_mutators(
                |node_ctx| { make_test_wallet_dest(node_ctx, 19, P2wpkh) },
                |_chan, tx, _input, _commit_num, _redeemscript, _amount_sat| {
                    tx.output[0].value = 1_000;
                },
            ),
            "policy failure: validate_fee: validate_delayed_sweep: \
             fee above maximum: 1978997 > 46000"
        );
    }
}
