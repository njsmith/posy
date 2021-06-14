mod cache;
mod net;
mod nuget;
mod prelude;
mod pypi;
mod resolve;
mod util;
#[forbid(unsafe_code)]
mod vocab;
//mod resolve;

use anyhow::Result;
//use std::io::Cursor;
use std::time::Duration;
use ureq::AgentBuilder;

use crate::prelude::*;
//use std::io::prelude::*;

use structopt::StructOpt;

#[derive(StructOpt)]
struct Opt {
    inputs: Vec<String>,
}

fn main() -> Result<()> {
    let opt = Opt::from_args();

    let agent = AgentBuilder::new()
        .timeout_read(Duration::from_secs(5))
        .timeout_write(Duration::from_secs(5))
        .build();

    // let nuget = nuget::Nuget::new(&agent)?;
    // println!("Python versions: {:?}", nuget.versions("python")?);

    // let body = nuget.get("python", "3.9.1")?;
    // let zipfile = Cursor::new(body);

    // let zip = zip::ZipArchive::new(zipfile)?;

    // for name in zip.file_names() {
    //     println!("Contains: {}", name);
    // }

    let pypi = crate::pypi::PyPI {
        agent: agent.clone(),
    };

    let root_reqs = opt
        .inputs
        .into_iter()
        .map(|s| Requirement::parse(&s, ParseExtra::NotAllowed))
        .collect::<Result<Vec<Requirement>>>()?;

    use std::cell::RefCell;
    let deps = resolve::PythonDependencies {
        pypi,
        known_artifacts: RefCell::new(HashMap::new()),
        known_metadata: RefCell::new(HashMap::new()),
        root_reqs,
    };

    println!("solution: {:?}", deps.resolve());

    Ok(())

    // let info = pypi.package_info(&"trio".try_into()?)?;
    // println!("{:?}", info);

    // let target = &info[0].artifacts[0].url;
    // println!("{}", target);
    // let mut body = Vec::<u8>::new();
    // agent.request_url("GET", &target).call()?.into_reader().read_to_end(&mut body)?;

    // println!("File length: {}", body.len());

    // let zipfile = Cursor::new(body);
    // let mut zip = zip::ZipArchive::new(zipfile)?;

    // // Was going to use this to compute the dist-info name, but that uses an
    // // unnormalized package name, so probably the wrong strategy...
    // let (_, filename) = target.path().rsplit_once("/").unwrap();
    // let wheelname: WheelName = filename.try_into()?;
    // println!("Fetched {:?}", wheelname);

    // let names: Vec<String> = zip.file_names().map(|s| s.to_owned()).collect();

    // for name in names {
    //     let zipfile = zip.by_name(&name)?;
    //     println!("Contains: {} @ {}", name, zipfile.data_start());
    // }

    // fn get<T: Read + Seek>(zip: &mut zip::ZipArchive<T>, file_name: &str) -> Result<Vec<u8>> {
    //     let mut buf = Vec::new();
    //     let mut zipfile = zip.by_name(file_name)?;
    //     println!("{} at {}", file_name, zipfile.data_start());
    //     zipfile.read_to_end(&mut buf)?;
    //     Ok(buf)
    // }

    // let names: Vec<String> = zip.file_names().map(|n| n.to_owned()).collect();

    // for file_name in names {
    //     if file_name.ends_with(".dist-info/WHEEL") {
    //         let wheel_metadata = WheelMetadata::parse(&get(&mut zip, &file_name)?)?;
    //         println!("{:?}", wheel_metadata);
    //     }
    //     if file_name.ends_with(".dist-info/METADATA") {
    //         let core_metadata = CoreMetadata::parse(&get(&mut zip, &file_name)?)?;
    //         println!("{:?}", core_metadata);
    //     }
    // }

    // Ok(())
}
