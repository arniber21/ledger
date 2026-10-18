//! SPL settlement of one fill on a local validator.
//!
//! Adapted from the Solana Cookbook token examples (Apache-2.0, Solana
//! Foundation), composed form:
//! - `code/token/transfer-token/transfer-token.en.ts` (`transferChecked`)
//! - `code/token/create-mint-account/create-mint-account.en.ts`
//! - `code/token/create-token-account/ata.en.ts`
//! - `code/token/mint-token/mint-token.en.ts`
//!
//! A fill pays the quote (USD) from the buyer to the seller. The fee payer and
//! the buyer's token-account owner both sign, matching that transfer example.
//! The cookbook's sample secret keys are not used. Keys here are generated.
//!
//! Instruction builders come from `spl-token-interface` and
//! `spl-associated-token-account-interface`, which the `spl-token` crate tells
//! clients to use. Mint initialization uses `initialize_mint2` so the
//! instruction does not require the rent sysvar.

use crate::types::{Fill, FillId, Side};
use ledger_core::AgentId;
use rust_decimal::Decimal;
use solana_client::rpc_client::RpcClient;
use solana_commitment_config::CommitmentConfig;
use solana_keypair::Keypair;
use solana_pubkey::Pubkey;
use solana_signer::Signer;
use solana_transaction::Transaction;
use spl_associated_token_account_interface::address::get_associated_token_address;
use spl_token_interface::instruction::transfer_checked;
use std::collections::HashMap;
use std::fmt;
use std::time::Duration;
use thiserror::Error;
#[cfg(test)]
use {
    solana_program_pack::Pack,
    solana_system_interface::instruction as system_instruction,
    spl_associated_token_account_interface::instruction::create_associated_token_account,
    spl_token_interface::instruction::{initialize_mint2, mint_to_checked},
    spl_token_interface::state::Mint,
};

/// JSON-RPC URL of the local `solana-test-validator` started by `just validator`.
pub const LOCAL_VALIDATOR_URL: &str = "http://127.0.0.1:8899";

/// Decimals on the quote mint. One dollar is 100 atoms.
pub const QUOTE_DECIMALS: u8 = 2;

/// A confirmed SPL transfer for one fill.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Settlement {
    /// Fill this transfer paid.
    pub fill_id: FillId,
    /// Confirmed transaction signature, base58.
    pub signature: String,
}

/// Why a fill was not settled.
#[derive(Debug, Error)]
#[non_exhaustive]
pub enum SettlementError {
    /// The buyer or seller has no token-account owner registered.
    #[error("unknown agent {agent}")]
    UnknownAgent {
        /// Agent id that was missing.
        agent: String,
    },
    /// The quote amount is not an integer number of token atoms.
    #[error("quote amount {amount} is not a whole number of token atoms")]
    AmountNotRepresentable {
        /// Quote amount that could not be converted.
        amount: String,
    },
    /// The quote amount does not fit in a `u64` atom count.
    #[error("quote amount overflowed")]
    AmountOverflow,
    /// The token program rejected the instruction before it was sent.
    #[error("spl instruction failed: {0}")]
    Program(#[from] solana_program_error::ProgramError),
    /// The validator RPC failed or could not be reached.
    #[error("validator rpc failed: {0}")]
    Rpc(#[from] solana_client::client_error::ClientError),
}

/// Settles fills as quote-asset SPL transfers.
///
/// The settler owns the fee payer and each trader's owner key. It does not
/// print those keys. Associated token accounts are derived from the owner and
/// the quote mint; they must already exist and the buyer must hold the quote.
pub struct TokenSettler {
    rpc: RpcClient,
    fee_payer: Keypair,
    mint: Pubkey,
    decimals: u8,
    traders: HashMap<AgentId, Keypair>,
}

impl fmt::Debug for TokenSettler {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let traders: Vec<&str> = self.traders.keys().map(AgentId::as_str).collect();
        f.debug_struct("TokenSettler")
            .field("mint", &self.mint)
            .field("decimals", &self.decimals)
            .field("traders", &traders)
            .finish_non_exhaustive()
    }
}

impl TokenSettler {
    /// Builds a settler aimed at `rpc_url`.
    ///
    /// `traders` maps an agent to the key that owns that agent's quote token
    /// account. The fee payer pays SOL for the transfer and is not logged.
    #[must_use]
    pub fn new(
        rpc_url: &str,
        fee_payer: Keypair,
        mint: Pubkey,
        decimals: u8,
        traders: impl IntoIterator<Item = (AgentId, Keypair)>,
    ) -> Self {
        Self::with_timeout(
            rpc_url,
            fee_payer,
            mint,
            decimals,
            traders,
            Duration::from_secs(30),
        )
    }

    /// Same as [`TokenSettler::new`] with an RPC timeout.
    ///
    /// A short timeout is how a dead validator fails quickly.
    #[must_use]
    pub fn with_timeout(
        rpc_url: &str,
        fee_payer: Keypair,
        mint: Pubkey,
        decimals: u8,
        traders: impl IntoIterator<Item = (AgentId, Keypair)>,
        timeout: Duration,
    ) -> Self {
        let traders = traders.into_iter().collect();
        let rpc = RpcClient::new_with_timeout_and_commitment(
            rpc_url.to_owned(),
            timeout,
            CommitmentConfig::confirmed(),
        );
        Self {
            rpc,
            fee_payer,
            mint,
            decimals,
            traders,
        }
    }

    /// Quote mint this settler transfers.
    #[must_use]
    pub fn mint(&self) -> Pubkey {
        self.mint
    }

    /// Settles one fill. The signature is recorded on the `exchange.settle` span.
    ///
    /// A bid pays the taker to the maker. An ask pays the maker to the taker.
    /// The amount is the fill's quote amount, in mint atoms.
    ///
    /// # Errors
    ///
    /// Returns [`SettlementError::UnknownAgent`] when the buyer or seller was
    /// not registered, [`SettlementError::AmountNotRepresentable`] or
    /// [`SettlementError::AmountOverflow`] when the quote amount is not a
    /// token amount, and [`SettlementError::Rpc`] when the validator rejects
    /// the transfer or cannot be reached. RPC failure emits a `rejected-fill`
    /// error event so the Sentry layer captures it.
    pub fn settle(&self, fill: &Fill) -> Result<Settlement, SettlementError> {
        let span = tracing::info_span!(
            "exchange.settle",
            sentry.name = "exchange.settle",
            sentry.op = "exchange.settle",
            fill.id = %fill.id,
            tx.signature = tracing::field::Empty,
        );
        let _entered = span.enter();
        match self.transfer(fill) {
            Ok(settlement) => {
                let signature_text = settlement.signature.clone();
                span.record("tx.signature", signature_text.as_str());
                tracing::info!(
                    fill.id = %fill.id,
                    tx.signature = %signature_text,
                    "fill settled"
                );
                Ok(settlement)
            }
            Err(err) => {
                reject_fill(fill, &err);
                Err(err)
            }
        }
    }

    /// Settles each fill, stopping at the first failure.
    ///
    /// Fills before the failure are already confirmed. Their signatures are
    /// returned only when every fill succeeds. Settle one fill at a time to
    /// keep a signature when a later fill fails.
    ///
    /// # Errors
    ///
    /// Returns the same errors as [`TokenSettler::settle`].
    pub fn settle_each(&self, fills: &[Fill]) -> Result<Vec<Settlement>, SettlementError> {
        let mut settled = Vec::with_capacity(fills.len());
        for fill in fills {
            settled.push(self.settle(fill)?);
        }
        Ok(settled)
    }

    fn transfer(&self, fill: &Fill) -> Result<Settlement, SettlementError> {
        let (buyer, seller) = payment_parties(fill);
        let buyer_keys = self
            .traders
            .get(buyer)
            .ok_or_else(|| SettlementError::UnknownAgent {
                agent: buyer.as_str().to_owned(),
            })?;
        let seller_keys =
            self.traders
                .get(seller)
                .ok_or_else(|| SettlementError::UnknownAgent {
                    agent: seller.as_str().to_owned(),
                })?;
        let amount = quote_atoms(fill.quote_amount, self.decimals)?;
        let source = get_associated_token_address(&buyer_keys.pubkey(), &self.mint);
        let destination = get_associated_token_address(&seller_keys.pubkey(), &self.mint);
        let instruction = transfer_checked(
            &spl_token_interface::id(),
            &source,
            &self.mint,
            &destination,
            &buyer_keys.pubkey(),
            &[],
            amount,
            self.decimals,
        )?;
        let signature =
            send_transaction(&self.rpc, &self.fee_payer, &[instruction], &[buyer_keys])?;
        Ok(Settlement {
            fill_id: fill.id,
            signature: signature.to_string(),
        })
    }
}

/// Buyer pays the seller. A crossing bid buys; a crossing ask sells.
fn payment_parties(fill: &Fill) -> (&AgentId, &AgentId) {
    match fill.taker_side {
        Side::Bid => (&fill.taker, &fill.maker),
        Side::Ask => (&fill.maker, &fill.taker),
    }
}

fn quote_atoms(amount: Decimal, decimals: u8) -> Result<u64, SettlementError> {
    if amount <= Decimal::ZERO {
        return Err(SettlementError::AmountNotRepresentable {
            amount: amount.to_string(),
        });
    }
    let scale = u32::from(decimals);
    let factor = 10u64
        .checked_pow(scale)
        .ok_or(SettlementError::AmountOverflow)?;
    let scaled = amount
        .checked_mul(Decimal::from(factor))
        .ok_or(SettlementError::AmountOverflow)?;
    if scaled.fract() != Decimal::ZERO {
        return Err(SettlementError::AmountNotRepresentable {
            amount: amount.to_string(),
        });
    }
    let mantissa = scaled.trunc().mantissa();
    u64::try_from(mantissa).map_err(|_| SettlementError::AmountOverflow)
}

fn reject_fill(fill: &Fill, err: &SettlementError) {
    tracing::error!(
        fill.id = %fill.id,
        error = %err,
        "rejected-fill"
    );
}

fn send_transaction(
    rpc: &RpcClient,
    fee_payer: &Keypair,
    instructions: &[solana_instruction::Instruction],
    extra_signers: &[&Keypair],
) -> Result<solana_signature::Signature, SettlementError> {
    let mut signers = Vec::with_capacity(1 + extra_signers.len());
    signers.push(fee_payer);
    for signer in extra_signers {
        if signer.pubkey() != fee_payer.pubkey() {
            signers.push(*signer);
        }
    }
    let blockhash = rpc.get_latest_blockhash()?;
    let transaction = Transaction::new_signed_with_payer(
        instructions,
        Some(&fee_payer.pubkey()),
        &signers,
        blockhash,
    );
    Ok(rpc.send_and_confirm_transaction(&transaction)?)
}

/// Creates the quote mint and associated token accounts used by a local test.
///
/// This is the cookbook's create-mint, ATA, and mint-to sequence. The fee
/// payer is the mint authority and pays for the accounts.
///
/// # Errors
///
/// Returns [`SettlementError::Rpc`] when the validator cannot confirm the
/// setup transactions, and [`SettlementError::Program`] when an instruction
/// cannot be built.
#[cfg(test)]
fn provision_quote_accounts(
    rpc_url: &str,
    fee_payer: &Keypair,
    traders: &[(&AgentId, &Keypair)],
    buyer: &AgentId,
    atoms: u64,
) -> Result<Pubkey, SettlementError> {
    let rpc = RpcClient::new_with_commitment(rpc_url.to_owned(), CommitmentConfig::confirmed());
    let airdrop = rpc.request_airdrop(&fee_payer.pubkey(), 2_000_000_000)?;
    rpc.poll_for_signature_with_commitment(&airdrop, CommitmentConfig::confirmed())?;

    let mint = Keypair::new();
    let lamports = rpc.get_minimum_balance_for_rent_exemption(Mint::LEN)?;
    let space = u64::try_from(Mint::LEN).map_err(|_| SettlementError::AmountOverflow)?;
    let create_mint = system_instruction::create_account(
        &fee_payer.pubkey(),
        &mint.pubkey(),
        lamports,
        space,
        &spl_token_interface::id(),
    );
    let init_mint = initialize_mint2(
        &spl_token_interface::id(),
        &mint.pubkey(),
        &fee_payer.pubkey(),
        None,
        QUOTE_DECIMALS,
    )?;
    send_transaction(&rpc, fee_payer, &[create_mint, init_mint], &[&mint])?;

    for (_, owner) in traders {
        let create_ata = create_associated_token_account(
            &fee_payer.pubkey(),
            &owner.pubkey(),
            &mint.pubkey(),
            &spl_token_interface::id(),
        );
        send_transaction(&rpc, fee_payer, &[create_ata], &[])?;
    }

    let buyer_owner = traders
        .iter()
        .find(|(agent, _)| *agent == buyer)
        .map(|(_, owner)| *owner)
        .ok_or_else(|| SettlementError::UnknownAgent {
            agent: buyer.as_str().to_owned(),
        })?;
    let buyer_ata = get_associated_token_address(&buyer_owner.pubkey(), &mint.pubkey());
    let mint_to = mint_to_checked(
        &spl_token_interface::id(),
        &mint.pubkey(),
        &buyer_ata,
        &fee_payer.pubkey(),
        &[],
        atoms,
        QUOTE_DECIMALS,
    )?;
    send_transaction(&rpc, fee_payer, &[mint_to], &[])?;
    Ok(mint.pubkey())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::telemetry::event_filter;
    use crate::types::{LimitOrder, OrderId, Price, Quantity};
    use crate::OrderBook;
    use solana_client::rpc_config::RpcTransactionConfig;
    use solana_transaction_status::UiTransactionEncoding;
    use std::ffi::OsStr;
    use std::path::PathBuf;
    use std::process::{Child, Command, Stdio};
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::sync::{Mutex, OnceLock};
    use std::thread;
    use std::time::Instant;
    use tracing_subscriber::prelude::*;

    fn price_cents(cents: i64) -> Price {
        Price::new(Decimal::new(cents, 2)).expect("positive price")
    }

    fn lots(units: i64) -> Quantity {
        Quantity::new(Decimal::new(units, 0)).expect("positive quantity")
    }

    fn agent(name: &str) -> AgentId {
        AgentId::new(name).expect("agent id")
    }

    fn order(id: u64, name: &str, side: Side, cents: i64, units: i64) -> LimitOrder {
        LimitOrder {
            id: OrderId::new(id),
            agent: agent(name),
            side,
            price: price_cents(cents),
            quantity: lots(units),
        }
    }

    #[test]
    fn quote_atoms_use_cents() {
        assert_eq!(quote_atoms(Decimal::new(2000, 2), 2).expect("20.00"), 2_000);
        assert!(matches!(
            quote_atoms(Decimal::new(20001, 3), 2).expect_err("extra digit"),
            SettlementError::AmountNotRepresentable { .. }
        ));
        assert!(matches!(
            quote_atoms(Decimal::ZERO, 2).expect_err("zero"),
            SettlementError::AmountNotRepresentable { .. }
        ));
    }

    #[test]
    fn bid_buyer_is_the_taker_and_ask_buyer_is_the_maker() {
        let mut book = OrderBook::new();
        book.submit(order(1, "seller", Side::Ask, 500, 4), None)
            .expect("resting ask");
        let bid = book
            .submit(order(2, "buyer", Side::Bid, 500, 4), None)
            .expect("crossing bid");
        let (buyer, seller) = payment_parties(&bid.fills[0]);
        assert_eq!(buyer.as_str(), "buyer");
        assert_eq!(seller.as_str(), "seller");

        let mut book = OrderBook::new();
        book.submit(order(1, "buyer", Side::Bid, 500, 4), None)
            .expect("resting bid");
        let ask = book
            .submit(order(2, "seller", Side::Ask, 500, 4), None)
            .expect("crossing ask");
        let (buyer, seller) = payment_parties(&ask.fills[0]);
        assert_eq!(buyer.as_str(), "buyer");
        assert_eq!(seller.as_str(), "seller");
    }

    #[test]
    fn killed_validator_rejects_the_fill_and_reports_a_sentry_error() {
        let buyer = agent("buyer");
        let seller = agent("seller");
        let settler = TokenSettler::with_timeout(
            "http://127.0.0.1:1",
            Keypair::new(),
            Pubkey::new_unique(),
            QUOTE_DECIMALS,
            [
                (buyer.clone(), Keypair::new()),
                (seller.clone(), Keypair::new()),
            ],
            Duration::from_secs(2),
        );
        let fill = sample_fill(&buyer, &seller);

        let seen = std::sync::Arc::new(AtomicBool::new(false));
        let seen_layer = RejectedFill(std::sync::Arc::clone(&seen));
        let sentry_layer = sentry::integrations::tracing::layer().event_filter(event_filter);
        let subscriber = tracing_subscriber::registry()
            .with(sentry_layer)
            .with(seen_layer);

        tracing::subscriber::with_default(subscriber, || {
            let events = sentry::test::with_captured_events(|| {
                let err = settler.settle(&fill).expect_err("dead validator");
                assert!(
                    matches!(err, SettlementError::Rpc(_)),
                    "expected an rpc failure, got {err}"
                );
            });
            assert!(
                events.iter().any(|event| {
                    event.level == sentry::protocol::Level::Error
                        && event
                            .message
                            .as_deref()
                            .is_some_and(|message| message.contains("rejected-fill"))
                }),
                "expected a Sentry error for rejected-fill, got {events:?}"
            );
        });
        assert!(
            seen.load(Ordering::SeqCst),
            "rejected-fill event was not emitted"
        );
    }

    #[test]
    fn fill_settles_as_a_confirmed_spl_transfer() {
        ensure_local_validator();
        let buyer = agent("buyer");
        let seller = agent("seller");
        let buyer_keys = Keypair::new();
        let seller_keys = Keypair::new();
        let fee_payer = Keypair::new();
        let mut book = OrderBook::new();
        book.submit(order(1, "seller", Side::Ask, 500, 4), None)
            .expect("resting ask");
        let report = book
            .submit(order(2, "buyer", Side::Bid, 500, 4), None)
            .expect("crossing bid");
        let fill = &report.fills[0];
        let atoms = quote_atoms(fill.quote_amount, QUOTE_DECIMALS).expect("atoms");

        let mint = provision_quote_accounts(
            LOCAL_VALIDATOR_URL,
            &fee_payer,
            &[(&buyer, &buyer_keys), (&seller, &seller_keys)],
            &buyer,
            atoms,
        )
        .expect("provision quote accounts");

        let seller_owner = seller_keys.pubkey();
        let settler = TokenSettler::new(
            LOCAL_VALIDATOR_URL,
            fee_payer,
            mint,
            QUOTE_DECIMALS,
            [(buyer, buyer_keys), (seller, seller_keys)],
        );
        let seller_ata = get_associated_token_address(&seller_owner, &mint);
        let settlement = settler.settle(fill).expect("settle fill");

        let signature = settlement
            .signature
            .parse::<solana_signature::Signature>()
            .expect("base58 signature");
        let rpc = RpcClient::new_with_commitment(
            LOCAL_VALIDATOR_URL.to_owned(),
            CommitmentConfig::confirmed(),
        );
        let fetched = rpc
            .get_transaction_with_config(
                &signature,
                RpcTransactionConfig {
                    encoding: Some(UiTransactionEncoding::Json),
                    commitment: Some(CommitmentConfig::confirmed()),
                    max_supported_transaction_version: Some(1),
                },
            )
            .expect("validator returns the signature");
        assert!(
            fetched
                .transaction
                .meta
                .as_ref()
                .and_then(|meta| meta.status.as_ref().err())
                .is_none(),
            "confirmed transaction failed on chain"
        );
        let balance = rpc
            .get_token_account_balance(&seller_ata)
            .expect("seller balance");
        assert_eq!(balance.amount, atoms.to_string());
        assert_eq!(settlement.fill_id, fill.id);
    }

    fn sample_fill(buyer: &AgentId, seller: &AgentId) -> Fill {
        let mut book = OrderBook::new();
        book.submit(
            LimitOrder {
                id: OrderId::new(1),
                agent: seller.clone(),
                side: Side::Ask,
                price: price_cents(500),
                quantity: lots(4),
            },
            None,
        )
        .expect("ask");
        let report = book
            .submit(
                LimitOrder {
                    id: OrderId::new(2),
                    agent: buyer.clone(),
                    side: Side::Bid,
                    price: price_cents(500),
                    quantity: lots(4),
                },
                None,
            )
            .expect("bid");
        report.fills.into_iter().next().expect("one fill")
    }

    struct RejectedFill(std::sync::Arc<AtomicBool>);

    impl<S> tracing_subscriber::Layer<S> for RejectedFill
    where
        S: tracing::Subscriber,
    {
        fn on_event(
            &self,
            event: &tracing::Event<'_>,
            _ctx: tracing_subscriber::layer::Context<'_, S>,
        ) {
            let mut message = String::new();
            event.record(&mut MessageVisitor(&mut message));
            if message.contains("rejected-fill") {
                self.0.store(true, Ordering::SeqCst);
            }
        }
    }

    struct MessageVisitor<'a>(&'a mut String);

    impl tracing::field::Visit for MessageVisitor<'_> {
        fn record_debug(&mut self, field: &tracing::field::Field, value: &dyn fmt::Debug) {
            if field.name() == "message" {
                self.0.push_str(&format!("{value:?}"));
            }
        }

        fn record_str(&mut self, field: &tracing::field::Field, value: &str) {
            if field.name() == "message" {
                self.0.push_str(value);
            }
        }
    }

    struct ValidatorProcess(Child);

    impl Drop for ValidatorProcess {
        fn drop(&mut self) {
            let _ = self.0.kill();
            let _ = self.0.wait();
        }
    }

    fn ensure_local_validator() {
        static STARTED: OnceLock<Mutex<Option<ValidatorProcess>>> = OnceLock::new();
        let slot = STARTED.get_or_init(|| Mutex::new(None));
        let mut guard = slot.lock().expect("validator lock");
        if validator_ready() {
            return;
        }
        let ledger = std::env::temp_dir().join(format!("ledger-settlement-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&ledger);
        std::fs::create_dir_all(&ledger).expect("validator ledger dir");
        let log_path = ledger.join("validator.log");
        let log = std::fs::File::create(&log_path).expect("validator log");
        let child = Command::new(validator_bin())
            .args([
                OsStr::new("--reset"),
                OsStr::new("--quiet"),
                OsStr::new("--rpc-port"),
                OsStr::new("8899"),
                OsStr::new("--gossip-port"),
                OsStr::new("18000"),
                OsStr::new("--faucet-port"),
                OsStr::new("9900"),
                OsStr::new("--dynamic-port-range"),
                OsStr::new("18010-18100"),
                OsStr::new("--ledger"),
            ])
            .arg(&ledger)
            .stdout(Stdio::null())
            .stderr(Stdio::from(log))
            .spawn()
            .expect("spawn solana-test-validator");
        *guard = Some(ValidatorProcess(child));
        let deadline = Instant::now() + Duration::from_secs(90);
        while Instant::now() < deadline {
            if validator_ready() {
                return;
            }
            if let Some(child) = guard.as_mut() {
                if let Some(status) = child.0.try_wait().expect("validator status") {
                    let log = std::fs::read_to_string(&log_path).unwrap_or_default();
                    panic!("solana-test-validator exited {status}: {log}");
                }
            }
            drop(guard);
            thread::sleep(Duration::from_millis(250));
            guard = slot.lock().expect("validator lock");
        }
        let log = std::fs::read_to_string(&log_path).unwrap_or_default();
        panic!("solana-test-validator did not become ready: {log}");
    }

    fn validator_ready() -> bool {
        let rpc =
            RpcClient::new_with_timeout(LOCAL_VALIDATOR_URL.to_owned(), Duration::from_secs(2));
        rpc.get_version().is_ok()
    }

    fn validator_bin() -> PathBuf {
        if let Some(path) = std::env::var_os("PATH") {
            for dir in std::env::split_paths(&path) {
                let candidate = dir.join("solana-test-validator");
                if candidate.is_file() {
                    return candidate;
                }
            }
        }
        let home = std::env::var("HOME").unwrap_or_default();
        PathBuf::from(home)
            .join(".local/share/solana/install/active_release/bin/solana-test-validator")
    }
}
