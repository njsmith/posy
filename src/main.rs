mod cache;
mod package_db;
mod prelude;
mod resolve;
mod util;
mod vocab;

mod brief;
mod platform_tags;
mod seek_slice;
#[cfg(test)]
mod test_util;

use anyhow::Result;

use crate::{brief::Brief, platform_tags::Platform, prelude::*};

use structopt::StructOpt;

#[derive(StructOpt)]
struct Opt {
    inputs: Vec<String>,
}

const ENV: Lazy<HashMap<String, String>> = Lazy::new(|| {
    // Copied from
    //   print(json.dumps(packaging.markers.default_environment(), sort_keys=True, indent=4))
    serde_json::from_str(
        r##"
        {
            "implementation_name": "cpython",
            "implementation_version": "3.8.6",
            "os_name": "posix",
            "platform_machine": "x86_64",
            "platform_python_implementation": "CPython",
            "platform_release": "5.8.0-53-generic",
            "platform_system": "Linux",
            "platform_version": "#60-Ubuntu SMP Thu May 6 07:46:32 UTC 2021",
            "python_full_version": "3.8.6",
            "python_version": "3.8",
            "sys_platform": "linux"
        }
        "##,
    )
    .unwrap()
});

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
    );
    println!("{:#?}", db.get_metadata::<Wheel, _>(
        db.artifacts_for_release(
            &"anyio".try_into().unwrap(),
            &"3.0.1".try_into().unwrap(),
        )
        .unwrap(),
    )
    .unwrap());
    let brief = Brief {
        python: "cpython_unofficial >= 3".try_into().unwrap(),
        requirements: vec![
            "sniffio < 1.1".try_into().unwrap(),
            "anyio".try_into().unwrap(),
        ],
    };
    let platform = Platform::from_core_tag("manylinux_2_17_x86_64");

    //brief.resolve(&db, &platform)?;

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
