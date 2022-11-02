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

use anyhow::Result;

use crate::{brief::Brief, platform_tags::Platform, prelude::*};

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
            "starlette".try_into().unwrap(),
            "scipy".try_into().unwrap(),
        ],
    };
    let platform = Platform::from_core_tag("manylinux_2_17_x86_64");

    let blueprint = brief.resolve(&db, &platform)?;

    println!("{}", blueprint);

    // let root_reqs = opt
    //     .inputs
    //     .into_iter()
    //     .map(|s| s.try_into())
    //     .collect::<Result<Vec<UserRequirement>>>()?;

    // let pins =
    //     resolve::resolve(&root_reqs, &*ENV, &index, &HashMap::new(), &|_| false)?;
    // for pin in pins {
    //     println!("{} v{}", pin.name.as_given(), pin.version);
    //     println!("   requirements from {}", pin.expected_requirements_source);
    //     //println!("   requirements: {:?}", pin.expected_requirements);
    // }

    Ok(())
}
