//! RESEARCH-ONLY. `algoglyph-contracts`: derive every AlgoGlyph kernel's contract from the pinned
//! AlgoGlyph source and write it to tools/research/schema/derived/algoglyph-contracts.json, or with
//! `--check` fail while the committed artifact differs (I0228). See src/algoglyph_contracts.rs.

#[path = "../algoglyph_contracts.rs"]
mod algoglyph_contracts;

use std::path::Path;

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let repo = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../..");
    std::process::exit(algoglyph_contracts::cli(&args, &repo, &|k| std::env::var(k).ok(), &mut std::io::stdout()));
}
