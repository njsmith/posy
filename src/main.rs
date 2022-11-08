mod package_db;
mod prelude;
mod resolve;
mod util;
mod vocab;
mod kvstore;

mod brief;
mod env;
mod platform_tags;
mod seek_slice;
#[cfg(test)]
mod test_util;
mod tree;
mod trampolines;

use std::path::Path;
use anyhow::Result;

use crate::{brief::Brief, prelude::*, env::EnvForest};

use structopt::StructOpt;

#[derive(StructOpt)]
struct Opt {
    inputs: Vec<String>,
}

fn main() -> Result<()> {
    let opt = Opt::from_args();

    //println!("user agent: {}", net::user_agent());
    // println!(
    //     "platform tags: {:?}",
    //     platform_tags::current_platform_tags()
    // );

    let db = package_db::PackageDB::new(
        &vec![
            Url::parse("https://pybi.vorpus.org")?,
            Url::parse("https://pypi.org/simple/")?,
        ],
        PROJECT_DIRS.cache_dir(),
    )?;

    // let pybi_ai = db
    //     .artifacts_for_release(
    //         &"cpython_unofficial".try_into().unwrap(),
    //         &"3.10.8".try_into().unwrap(),
    //     )
    //     .unwrap();
    // let pybi = db.get_artifact::<Pybi>(&pybi_ai[0]).unwrap();
    // _ = std::fs::remove_dir_all("/tmp/unpack-test");
    // std::fs::create_dir_all("/tmp/unpack-test")?;
    // pybi.unpack(&std::path::Path::new("/tmp/unpack-test"))?;

    let brief = Brief {
        python: "cpython_unofficial >= 3".try_into().unwrap(),
        requirements: vec![
            "trio".try_into().unwrap(),
            "numpy".try_into().unwrap(),
            "black".try_into().unwrap(),
        ],
    };
    let platform = PybiPlatform::current_platform()?;

    let blueprint = brief.resolve(&db, &platform)?;

    let env_forest = EnvForest::new(Path::new("/tmp/posy-test-forest"))?;

    let env = env_forest.get_env(&db, &blueprint, &platform)?;

    let old_path = std::env::var_os("PATH").ok_or(anyhow!("no $PATH?"))?;
    let mut new_paths = env.bin_dirs.clone();
    new_paths.extend(std::env::split_paths(&old_path));
    let new_path = std::env::join_paths(&new_paths)?;

    let mut child = std::process::Command::new("python")
        .env("PATH", new_path)
        .env("POSY_PYTHON", env.python.as_os_str())
        .env("POSY_PYTHONW", env.pythonw.as_os_str())
        .env("POSY_PYTHON_PACKAGES", std::env::join_paths(&env.lib_dirs)?)
        .spawn()?;
    child.wait()?;

    Ok(())
}
