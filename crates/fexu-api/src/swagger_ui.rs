//! Swagger UI 5.29.0, served from this crate.
//!
//! The files in `assets/swagger-ui` are the `swagger-ui-dist@5.29.0` release
//! (Apache License 2.0). The page does not load a third-party host. `validatorUrl`
//! stays null, and `queryConfigEnabled` stays false so a query string cannot
//! point the UI at another document or turn try-it-out back on.

pub(crate) const VERSION: &str = "5.29.0";

const BUNDLE: &[u8] = include_bytes!("../assets/swagger-ui/swagger-ui-bundle.js");
const CSS: &[u8] = include_bytes!("../assets/swagger-ui/swagger-ui.css");
const LICENSE: &[u8] = include_bytes!("../assets/swagger-ui/LICENSE");
const NOTICE: &[u8] = include_bytes!("../assets/swagger-ui/NOTICE");

pub(crate) fn asset_suffixes() -> &'static [&'static str] {
    &[
        "swagger-ui-bundle.js",
        "swagger-ui.css",
        "swagger-initializer.js",
        "LICENSE",
        "NOTICE",
    ]
}

pub(crate) fn asset(name: &str) -> Option<(&'static [u8], &'static str)> {
    match name {
        "swagger-ui-bundle.js" => Some((BUNDLE, "text/javascript; charset=utf-8")),
        "swagger-ui.css" => Some((CSS, "text/css; charset=utf-8")),
        "LICENSE" | "NOTICE" => {
            let body = if name == "LICENSE" { LICENSE } else { NOTICE };
            Some((body, "text/plain; charset=utf-8"))
        }
        _ => None,
    }
}

pub(crate) fn page(title: &str, ui_path: &str) -> String {
    let title = escape_html(title);
    let ui_path = escape_html(ui_path);
    format!(
        r#"<!DOCTYPE html>
<html lang="en">
<head>
<meta charset="utf-8">
<meta name="viewport" content="width=device-width, initial-scale=1">
<title>{title} API</title>
<link rel="stylesheet" href="{ui_path}/swagger-ui.css">
</head>
<body>
<div id="swagger-ui"></div>
<!-- Swagger UI {version}, Apache License 2.0. License text: {ui_path}/LICENSE -->
<script src="{ui_path}/swagger-ui-bundle.js"></script>
<script src="{ui_path}/swagger-initializer.js"></script>
</body>
</html>
"#,
        version = VERSION,
    )
}

pub(crate) fn initializer(schema_path: &str, try_it_out: bool) -> String {
    let url = serde_json::to_string(schema_path).expect("schema path is a string");
    let enabled = if try_it_out { "true" } else { "false" };
    let methods = if try_it_out {
        r#"["get", "post", "put", "patch", "delete"]"#
    } else {
        "[]"
    };
    format!(
        r##""use strict";
window.ui = SwaggerUIBundle({{
  url: {url},
  dom_id: "#swagger-ui",
  deepLinking: false,
  presets: [SwaggerUIBundle.presets.apis],
  layout: "BaseLayout",
  docExpansion: "list",
  defaultModelsExpandDepth: 1,
  validatorUrl: null,
  queryConfigEnabled: false,
  persistAuthorization: false,
  withCredentials: true,
  tryItOutEnabled: {enabled},
  supportedSubmitMethods: {methods},
  requestInterceptor: function (request) {{
    request.credentials = "same-origin";
    return request;
  }}
}});
"##
    )
}

fn escape_html(value: &str) -> String {
    value
        .chars()
        .map(|char| match char {
            '&' => "&amp;".to_owned(),
            '<' => "&lt;".to_owned(),
            '>' => "&gt;".to_owned(),
            '"' => "&quot;".to_owned(),
            _ => char.to_string(),
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn swagger_page_stays_on_this_origin() {
        let html = page("Shop <api>", "/api/docs");
        assert!(html.contains("Shop &lt;api&gt;"));
        assert!(html.contains("src=\"/api/docs/swagger-ui-bundle.js\""));
        assert!(html.contains("Apache License 2.0"));
        assert!(!html.contains("<script>"));
        assert!(!html.to_ascii_lowercase().contains("unpkg"));
        assert!(!html.to_ascii_lowercase().contains("cdn"));
        let script = initializer("/api/openapi.json", false);
        assert!(script.contains("validatorUrl: null"));
        assert!(script.contains("queryConfigEnabled: false"));
        assert!(script.contains("tryItOutEnabled: false"));
        assert!(script.contains("supportedSubmitMethods: []"));
        assert!(script.contains("credentials = \"same-origin\""));
        assert!(script.contains("persistAuthorization: false"));
        assert!(!script.contains("headers[\"Authorization\"]"));
        assert!(initializer("/api/openapi.json", true).contains("tryItOutEnabled: true"));
        assert!(std::str::from_utf8(LICENSE)
            .unwrap()
            .contains("Apache License"));
        assert!(std::str::from_utf8(BUNDLE)
            .unwrap()
            .contains("SwaggerUIBundle"));
        assert_eq!(asset("missing"), None);
    }
}
