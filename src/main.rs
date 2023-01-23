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

    let env_forest = EnvForest::new(Path::new("posy-test-forest"))?;
    let build_tmp = tempfile::TempDir::new()?;
    let build_store = KVDirStore::new(build_tmp.path())?;

    let db = package_db::PackageDB::new(
        &vec![
            Url::parse("https://pybi.vorpus.org")?,
            Url::parse("https://pypi.org/simple/")?,
        ],
        PROJECT_DIRS.cache_dir(),
        // PackageDB needs a place to install packages, in case it has to build some
        // sdists. Using a shared env_forest is efficient, because it means different
        // builds can share the same package installs.
        &env_forest,
        // This is the temporary directory we use for sdist builds. It's also a
        // content-addressed store, so if we want to build the same package twice (e.g.
        // first to get metadata, and then to get a wheel), we can re-use the same build
        // directory.
        &build_store,
    )?;
    // We can resolve and install for arbitrary platforms. But for this demo we'll just
    // use the platform of the machine we're running on. Or platforms, in case it
    // supports several (e.g. macOS arm64+x86_64, Windows 32bit+64bit, Linux
    // manylinux+musllinux, etc.).
    let platforms = PybiPlatform::native_platforms()?;

    // A "brief" is a user-level description of a desired environment.
    //   https://en.wikipedia.org/wiki/Brief_(architecture)
    let brief = Brief {
        // "cpython_unofficial" is the package name I used for my test pybis at
        // pybi.vorpus.org. We restrict to 3.10 or earlier because peewee upstream is
        // broken on 3.11 (it attempts to use the now-private longintrepr.h)
        python: "cpython_unofficial >= 3, < 3.11".try_into().unwrap(),
        requirements: vec![
            // Simple pure-Python package with some dependencies
            "trio".try_into().unwrap(),
            // Package with binary wheels
            "numpy".try_into().unwrap(),
            // Package with entrypoint scripts
            "black".try_into().unwrap(),
            // Package with no wheels, only sdist
            "peewee".try_into().unwrap(),
        ],
        allow_pre: AllowPre::Some(HashSet::new()),
    };
    // A "blueprint" is a set of fully-resolved package pins describing an environment,
    // like a lock-file.
    let blueprint = brief.resolve(&db, &platforms, None, &[])?;

    // And an "env" of course is an installed environment.
    let env = env_forest.get_env(&db, &blueprint, &platforms, &[])?;

    let mut cmd = std::process::Command::new("python");
    // env.env_vars() gives us the magic environment variables needed to run a command
    // in our new environment.
    cmd.envs(env.env_vars()?);

    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt;
        Err(cmd.exec())?;
        unreachable!();
    }
    #[cfg(windows)]
    {
        // XX FIXME: factor out the windows trampoline code and reuse it here.
        //
        // unwrap() is safe b/c this branch only runs on windows, and Windows doesn't
        // have special exit statuses; that's a special thing for Unix signals.
        std::process::exit(cmd.status()?.code().unwrap());
    }
    #[cfg(not(any(unix, windows)))]
    {
        not_supported
    }
}
