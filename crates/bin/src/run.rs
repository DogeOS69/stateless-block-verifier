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
    /// Required from Tsuki onwards, where the base-fee rule needs the parent of the first block.
    #[arg(long)]
    parent_header: Option<PathBuf>,
}

impl RunFileCommand {
    pub fn run(self) -> eyre::Result<()> {
        let parent = self.parent_header.as_deref().map(read_header).transpose()?;

        // Witnesses are replayed in block order so that each one is anchored to the previous
        // one's header, whatever order the shell expanded the paths in.
        let mut witnesses = self
            .path
            .iter()
            .map(|path| read_witness(path).map(|witness| (path.clone(), witness)))
            .collect::<eyre::Result<Vec<_>>>()?;
        witnesses.sort_by_key(|(_, witness)| witness.header.number);

        let mut gas_used = 0;
        let mut parent = parent;
        for (path, witness) in witnesses {
            let header = witness.header.clone();
            gas_used += run_witness(&path, witness, parent.as_ref(), self.hardfork)?.gas_used;
            parent = Some(header);
        }
        dev_info!("Gas used: {}", gas_used);

        Ok(())
    }
}

fn read_witness(path: &Path) -> eyre::Result<BlockWitness> {
    let witness = std::fs::File::open(path)?;
    let jd = &mut serde_json::Deserializer::from_reader(&witness);
    let witness = serde_path_to_error::deserialize::<_, BlockWitness>(jd)?;
    Ok(witness)
}

fn read_header(path: &Path) -> eyre::Result<Header> {
    let header = std::fs::File::open(path)?;
    let jd = &mut serde_json::Deserializer::from_reader(&header);
    Ok(serde_path_to_error::deserialize::<_, Header>(jd)?)
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
        dev_info!("Overriding hardfork to: {hardfork:?}");
        build_chain_spec_force_hardfork(chain, hardfork)
    } else {
        get_chain_spec(chain).context("chain not support")?
    };
    verify_catch_panics(witness, parent, chain_spec).inspect(|_| dev_info!("verified"))
}
