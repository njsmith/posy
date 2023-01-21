mod kvstore;
mod package_db;
mod prelude;
mod resolve;
mod util;
mod vocab;

mod env;
pub mod error;
mod output;
mod platform_tags;
mod seek_slice;
#[cfg(test)]
mod test_util;
mod trampolines;
mod tree;

use std::path::Path;

use crate::{env::EnvForest, prelude::*, resolve::Brief};

use clap::Parser;
use kvstore::KVDirStore;
use resolve::AllowPre;

#[derive(Parser)]
#[command(author, version, about)]
struct Cli {
    #[command(flatten)]
    output_args: output::OutputArgs,
}

fn main() -> Result<()> {
    let cli = Cli::parse();
    output::init(&cli.output_args);

    let env_forest = EnvForest::new(Path::new("/tmp/posy-test-forest"))?;
    let build_tmp = tempfile::TempDir::new()?;
    let build_store = KVDirStore::new(build_tmp.path())?;

    let db = package_db::PackageDB::new(
        &vec![
            Url::parse("https://pybi.vorpus.org")?,
            Url::parse("https://pypi.org/simple/")?,
        ],
        PROJECT_DIRS.cache_dir(),
        &env_forest,
        &build_store,
    )?;
    let platforms = PybiPlatform::native_platforms()?;

    let brief = Brief {
        // peewee is broken on 3.11
        python: "cpython_unofficial >= 3, < 3.11".try_into().unwrap(),
        requirements: vec![
            "trio".try_into().unwrap(),
            "numpy".try_into().unwrap(),
            "black".try_into().unwrap(),
        ],
        allow_pre: AllowPre::Some(HashSet::new()),
    };
    let blueprint = brief.resolve(&db, &platforms, None, &[])?;

    let env = env_forest.get_env(&db, &blueprint, &platforms, &[])?;

    let mut cmd = std::process::Command::new("python");
    cmd.envs(env.env_vars()?);

    if cfg!(unix) {
        use std::os::unix::process::CommandExt;
        Err(cmd.exec())?;
        unreachable!();
    } else {
        // XX FIXME: factor out the windows trampoline code and reuse it here.
        //
        // unwrap() is safe b/c this branch only runs on windows, and Windows doesn't
        // have special exit statuses; that's a special thing for Unix signals.
        std::process::exit(cmd.status()?.code().unwrap());
    }
}
