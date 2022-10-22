// Example pip user-agent:
//
//  pip/21.1.1 {"ci":null,"cpu":"x86_64","distro":{"id":"groovy","libc":{"lib":"glibc","version":"2.32"},"name":"Ubuntu","version":"20.10"},"implementation":{"name":"CPython","version":"3.8.6"},"installer":{"name":"pip","version":"21.1.1"},"openssl_version":"OpenSSL 1.1.1f  31 Mar 2020","python":"3.8.6","setuptools_version":"46.4.0","system":{"name":"Linux","release":"5.8.0-53-generic"}}
//
// See pip/_internal/network/session.py for details

use serde_json::json;

const CI_ENVIRONMENT_VARIABLES: &[&str] =
    &["BUILD_BUILDID", "BUILD_ID", "CI", "PIP_IS_CI"];

fn looks_like_ci() -> Option<bool> {
    // Either 'true' or 'null'
    if CI_ENVIRONMENT_VARIABLES
        .iter()
        .any(|name| std::env::var_os(name).is_some())
    {
        Some(true)
    } else {
        None
    }
}

pub fn user_agent() -> String {
    let installer = env!("CARGO_PKG_NAME");
    let version = env!("CARGO_PKG_VERSION");
    let data = json!({
        "installer": {
            "name": &installer,
            "version": &version,
        },
        "ci": looks_like_ci(),
        "cpu": std::env::consts::ARCH,
        "user_data": std::env::var("PIP_USER_AGENT_USER_DATA").ok(),
    });

    format!(
        "{}/{} {}",
        installer,
        version,
        serde_json::to_string(&data).unwrap(),
    )
}
