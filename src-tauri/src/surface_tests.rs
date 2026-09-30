//! Source-level acceptance checks for the desktop/native execution boundary.
//!
//! These deliberately inspect the checked-in sources rather than starting a
//! Tauri window. They catch command-registration drift and accidental return
//! of the old localhost/server integration without coupling tests to a GUI.

#[cfg(test)]
mod tests {
    use std::fs;
    use std::path::PathBuf;

    fn repo_file(relative: &str) -> String {
        let mut path = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
        path.push("..");
        path.push(relative);
        fs::read_to_string(path).expect("surface-test source file exists")
    }

    #[test]
    fn every_api_invoke_command_is_registered_in_tauri_handler() {
        let api = repo_file("desktop/src/lib/api.ts");
        let lib = repo_file("src-tauri/src/lib.rs");
        let mut commands = Vec::new();
        for marker in ["call<", "callValidated<"] {
            let mut rest = api.as_str();
            while let Some(index) = rest.find(marker) {
                rest = &rest[index + marker.len()..];
                if let Some(start) = rest.find("(\"") {
                    let value = &rest[start + 2..];
                    if let Some(end) = value.find('\"') {
                        commands.push(value[..end].to_string());
                        rest = &value[end..];
                    } else {
                        break;
                    }
                }
            }
        }
        commands.sort();
        commands.dedup();
        let missing: Vec<String> = commands
            .into_iter()
            .filter(|command| {
                !lib.contains(&format!("commands::{command}"))
                    && !lib.contains(&format!("commands::runs::{command}"))
            })
            .collect();
        assert!(
            missing.is_empty(),
            "api commands missing from generate_handler!: {missing:?}"
        );
    }

    #[test]
    fn desktop_has_no_localhost_http_or_websocket_execution_path() {
        let desktop = repo_file("desktop/src/lib/api.ts");
        assert!(
            !desktop.contains("fetch("),
            "desktop API must remain invoke-only"
        );
        assert!(
            !desktop.contains("WebSocket"),
            "desktop API must remain event-channel-only"
        );
        let all_desktop = repo_file("desktop/src/App.tsx") + &repo_file("desktop/src/lib/tauri.ts");
        assert!(
            !all_desktop.contains("localhost"),
            "desktop must not call a localhost service"
        );
    }

    #[test]
    fn tauri_backend_has_no_http_listener() {
        let tauri = repo_file("src-tauri/src/lib.rs") + &repo_file("src-tauri/src/commands/mod.rs");
        for marker in [
            "TcpListener",
            "axum::Server",
            "warp::serve",
            "actix_web::HttpServer",
        ] {
            assert!(
                !tauri.contains(marker),
                "unexpected HTTP listener marker: {marker}"
            );
        }
    }
}
