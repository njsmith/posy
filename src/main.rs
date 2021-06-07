mod metadata;

use anyhow::Result;
use std::io::Cursor;
use std::time::Duration;
use ureq::AgentBuilder;

mod nuget_api {
    const ROOT_URL: &str = "https://api.nuget.org/v3/index.json";

    use anyhow::Result;
    use serde::Deserialize;
    use std::io::Read;
    use std::vec::Vec;
    use ureq::Agent;

    #[derive(Deserialize, Debug)]
    struct Resource {
        #[serde(rename = "@id")]
        id: url::Url,
        #[serde(rename = "@type")]
        type_: String,
    }

    #[derive(Deserialize, Debug)]
    struct ResourcesPage {
        resources: Vec<Resource>,
    }

    #[derive(Deserialize, Debug)]
    struct VersionsPage {
        versions: Vec<String>,
    }

    impl ResourcesPage {
        fn get(agent: &Agent) -> Result<ResourcesPage> {
            let response = agent.get(ROOT_URL).call()?;
            Ok(response.into_json()?)
        }

        fn find(self, type_: &str) -> Result<url::Url> {
            for resource in self.resources {
                if resource.type_ == type_ {
                    return Ok(resource.id);
                }
            }
            Err(anyhow::anyhow!("Can't find resource type {:?}", type_))
        }
    }

    pub struct Nuget {
        agent: Agent,
        package_base_address: url::Url,
    }

    impl Nuget {
        pub fn new(agent: &Agent) -> Result<Nuget> {
            let agent = agent.clone();
            let resources = ResourcesPage::get(&agent)?;
            let package_base_address = resources.find("PackageBaseAddress/3.0.0")?;
            Ok(Nuget {
                agent,
                package_base_address,
            })
        }

        pub fn versions(&self, pkg: &str) -> Result<Vec<String>> {
            let trailing = format!("{}/index.json", pkg);
            let url = self.package_base_address.join(&trailing)?;
            let v: VersionsPage = self.agent.request_url("GET", &url).call()?.into_json()?;
            Ok(v.versions)
        }

        pub fn get(&self, pkg: &str, version: &str) -> Result<Vec<u8>> {
            let trailing = format!("{}/{}/{}.{}.nupkg", pkg, version, pkg, version);
            let url = self.package_base_address.join(&trailing)?;
            let response = self.agent.request_url("GET", &url).call()?;
            let mut body = Vec::<u8>::new();
            response.into_reader().read_to_end(&mut body)?;
            Ok(body)
        }
    }
}

fn main() -> Result<()> {
    println!("Hello, world!");
    let agent = AgentBuilder::new()
        .timeout_read(Duration::from_secs(5))
        .timeout_write(Duration::from_secs(5))
        .build();

    let nuget = nuget_api::Nuget::new(&agent)?;
    println!("Python versions: {:?}", nuget.versions("python")?);

    let body = nuget.get("python", "3.9.1")?;
    let zipfile = Cursor::new(body);

    let zip = zip::ZipArchive::new(zipfile)?;

    for name in zip.file_names() {
        println!("Contains: {}", name);
    }

    Ok(())
}
