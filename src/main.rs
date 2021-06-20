#![forbid(unsafe_code)]

mod cache;
mod net;
mod package_index;
mod prelude;
mod resolve;
mod util;
mod vocab;

#[cfg(test)]
mod test_util;

use anyhow::Result;
use std::time::Duration;
use ureq::AgentBuilder;

use crate::prelude::*;

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

    let agent = AgentBuilder::new()
        .timeout_read(Duration::from_secs(5))
        .timeout_write(Duration::from_secs(5))
        .build();

    let cache: cache::Cache = Default::default();

    let net = net::Net { agent, cache };

    let index = package_index::PackageIndex {
        net: net.clone(),
        base_url: package_index::ROOT_URL.clone(),
    };

    let root_reqs = opt
        .inputs
        .into_iter()
        .map(|s| Requirement::parse(&s, ParseExtra::NotAllowed))
        .collect::<Result<Vec<Requirement>>>()?;

    let pins =
        resolve::resolve(&root_reqs, &*ENV, &index, &HashMap::new(), &|_| false)?;
    for pin in pins {
        println!("{} v{}", pin.name.as_given(), pin.version);
        println!("   requirements from {}", pin.expected_requirements_source);
        println!("   requirements: {:?}", pin.expected_requirements);
    }

    Ok(())
}
