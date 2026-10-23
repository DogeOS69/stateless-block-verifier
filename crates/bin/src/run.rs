use crate::helpers::verifier::*;
use clap::Args;
use eyre::ContextCompat;
use sbv::{
    core::{verifier::VerifyResult, witness::BlockWitness},
    primitives::{
        chainspec::{Chain, build_chain_spec_force_hardfork, get_chain_spec},
        hardforks::Hardfork,
        types::Header,
    },
};
use std::path::{Path, PathBuf};

#[derive(Args, Debug)]
pub struct RunFileCommand {
    /// Path to the witness file
    #[arg(default_value = "witness.json")]
    path: Vec<PathBuf>,
    /// Hardfork
    #[arg(long, value_parser = clap::value_parser!(Hardfork))]
    hardfork: Option<Hardfork>,
    /// Path to a JSON block header trusted as the parent of the lowest witness.
    ///
    /// Anchoring is required for any chain that schedules Tsuki, where the first block's base fee
    /// and the chunk's previous proof endpoint both come from this header. The file takes either
    /// the shape a witness's own `header` object has, so a witness's header can be copied
    /// straight out of it, or `eth_getBlockByNumber`'s camelCase shape.
    ///
    /// It anchors the lowest witness given, ordered by chain id and then by block number, so pass
    /// it with the witnesses of one chain rather than with a glob over several.
    #[arg(long)]
    parent_header: Option<PathBuf>,
}

/// A trusted parent header in the shape a witness's own `header` object has: the field names of
/// the struct itself (`parent_hash`, `base_fee_per_gas`, …), as `serde_bincode_compat` writes them.
#[serde_with::serde_as]
#[derive(serde::Deserialize)]
#[serde(transparent)]
struct WitnessShapedHeader(
    #[serde_as(as = "sbv::primitives::types::consensus::serde_bincode_compat::Header")] Header,
);

/// Reads a trusted parent header, accepting either shape a user is likely to have to hand.
///
/// A witness writes its header with the struct's own field names; `eth_getBlockByNumber` writes
/// JSON-RPC's camelCase (`parentHash`, `baseFeePerGas`, …). Both are tried, and a file that is
/// neither reports what each attempt made of it: an untagged enum would collapse both paths into
/// "data did not match any variant", which says nothing about the field that is wrong.
fn parse_parent_header(json: &[u8]) -> eyre::Result<Header> {
    let witness_error = {
        let jd = &mut serde_json::Deserializer::from_slice(json);
        match serde_path_to_error::deserialize::<_, WitnessShapedHeader>(jd) {
            Ok(header) => return Ok(header.0),
            Err(error) => error,
        }
    };
    let jd = &mut serde_json::Deserializer::from_slice(json);
    serde_path_to_error::deserialize::<_, Header>(jd).map_err(|rpc_error| {
        eyre::eyre!(
            "the parent header is neither a witness header ({witness_error}) nor an RPC block \
             header ({rpc_error})"
        )
    })
}

impl RunFileCommand {
    pub fn run(self) -> eyre::Result<()> {
        let parent = self.parent_header.as_deref().map(read_header).transpose()?;

        let mut witnesses = self
            .path
            .iter()
            .map(|path| read_witness(path).map(|witness| (path.clone(), witness)))
            .collect::<eyre::Result<Vec<_>>>()?;
        // Witnesses are replayed in block order so each one can be anchored to the previous one's
        // header, whatever order the shell expanded the paths in.
        witnesses.sort_by_key(|(_, witness)| (witness.chain_id, witness.header.number));

        let mut gas_used = 0;
        // Only a witness that directly continues the previous one is anchored to it. A set of
        // unrelated witnesses - several chains, or a gap - verifies each on its own, as it did
        // before anchoring existed, rather than reporting a parent mismatch that says nothing
        // about the files.
        let mut previous: Option<&BlockWitness> = None;
        let mut parent = parent;
        for (path, witness) in &witnesses {
            let anchor = match previous {
                Some(previous) if continues(previous, witness) => Some(previous.header.clone()),
                Some(_) => None,
                None => parent.take(),
            };
            gas_used +=
                run_witness(path, witness.clone(), anchor.as_ref(), self.hardfork)?.gas_used;
            previous = Some(witness);
        }
        dev_info!("Gas used: {}", gas_used);

        Ok(())
    }
}

/// Whether `witness` is the block directly after `previous` on the same chain.
fn continues(previous: &BlockWitness, witness: &BlockWitness) -> bool {
    previous.chain_id == witness.chain_id
        && previous.header.number.checked_add(1) == Some(witness.header.number)
}

fn read_witness(path: &Path) -> eyre::Result<BlockWitness> {
    let witness = std::fs::File::open(path)?;
    let jd = &mut serde_json::Deserializer::from_reader(&witness);
    let witness = serde_path_to_error::deserialize::<_, BlockWitness>(jd)?;
    Ok(witness)
}

fn read_header(path: &Path) -> eyre::Result<Header> {
    parse_parent_header(&std::fs::read(path)?)
}

#[cfg_attr(feature = "dev", tracing::instrument(skip_all, fields(path = %path.display()), err))]
fn run_witness(
    path: &Path,
    witness: BlockWitness,
    parent: Option<&Header>,
    hardfork: Option<Hardfork>,
) -> eyre::Result<VerifyResult> {
    let chain = Chain::from_id(witness.chain_id);
    let chain_spec = if let Some(hardfork) = hardfork {
        // Forcing a fork below the chain's own replaces the rules the witness was produced under:
        // below Tsuki it drops the parent requirement, the base-fee rule and the authenticated
        // message-queue index, and it changes the EIP-1559 parameters.
        dev_info!(
            "Overriding hardfork to {hardfork:?}: the rules of the chain's own schedule do not apply"
        );
        build_chain_spec_force_hardfork(chain, hardfork)
    } else {
        get_chain_spec(chain).context("chain not support")?
    };
    verify_catch_panics(witness, parent, chain_spec).inspect(|_| dev_info!("verified"))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn witness(json: &str) -> BlockWitness {
        serde_json::from_str(json).unwrap()
    }

    /// A glob over a directory holding several chains must not be chained into one run. Before
    /// anchoring existed such a glob verified each witness on its own, and it still has to.
    #[test]
    fn only_the_directly_following_block_of_the_same_chain_is_an_anchor() {
        let a = witness(include_str!(
            "../../../testdata/scroll/feynman/534352-19604670.json"
        ));
        let b = witness(include_str!(
            "../../../testdata/scroll/feynman/534352-19604671.json"
        ));
        let c = witness(include_str!(
            "../../../testdata/scroll/feynman/534352-19604672.json"
        ));
        let other_chain = witness(include_str!(
            "../../../testdata/scroll/feynman/5343513301-1.json"
        ));
        assert_ne!(a.chain_id, other_chain.chain_id);

        assert!(continues(&a, &b));
        assert!(continues(&b, &c));
        // A gap, a repeat, the wrong direction and a different chain are all unanchored.
        assert!(!continues(&a, &c));
        assert!(!continues(&a, &a));
        assert!(!continues(&b, &a));
        assert!(!continues(&a, &other_chain));
        assert!(!continues(&other_chain, &a));
    }

    /// A witness's own `header` object is the obvious thing to copy into a `--parent-header` file,
    /// and an RPC block is the other. Both have to parse, because they disagree on field names.
    #[test]
    fn a_parent_header_file_takes_either_shape() {
        let witness: serde_json::Value = serde_json::from_str(include_str!(
            "../../../testdata/scroll/feynman/534352-19604670.json"
        ))
        .unwrap();
        let witness_shape = witness.get("header").expect("a witness carries its header");
        assert!(
            witness_shape.get("parent_hash").is_some(),
            "the witness shape is the struct's own field names"
        );

        let header = parse_parent_header(&serde_json::to_vec(witness_shape).unwrap())
            .expect("the witness shape must parse");
        assert_eq!(header.number, 19_604_670);
        assert!(header.base_fee_per_gas.is_some());

        // The same header in JSON-RPC's camelCase, as `eth_getBlockByNumber` returns it.
        let rpc_shape = serde_json::to_value(&header).unwrap();
        assert!(
            rpc_shape.get("parentHash").is_some(),
            "alloy's own encoding is camelCase"
        );
        assert_eq!(
            parse_parent_header(&serde_json::to_vec(&rpc_shape).unwrap())
                .expect("the RPC shape must parse"),
            header
        );
    }

    /// A file that is neither shape has to say what each attempt made of it, so the user knows
    /// which field to look at rather than only that nothing matched.
    #[test]
    fn a_parent_header_file_of_neither_shape_reports_both_attempts() {
        let error = parse_parent_header(br#"{"parent_hash": "not a hash"}"#)
            .expect_err("neither shape accepts this")
            .to_string();

        assert!(error.contains("witness header"), "{error}");
        assert!(error.contains("RPC block header"), "{error}");
        assert!(error.contains("parent_hash"), "{error}");
    }
}
