use std::cell::RefCell;
use std::rc::Rc;

use deno_core::{
    ModuleLoadOptions, ModuleLoadReferrer, ModuleLoadResponse, ModuleLoader, ModuleSource,
    ModuleSourceCode, ModuleSpecifier, ModuleType, ResolutionKind,
};
use deno_error::JsErrorBox;

use super::permissions::PermissionChecker;

/// Custom module loader that enforces permissions and rewrites npm: specifiers.
pub struct WfeModuleLoader {
    permissions: Rc<RefCell<PermissionChecker>>,
}

impl WfeModuleLoader {
    pub fn new(permissions: Rc<RefCell<PermissionChecker>>) -> Self {
        Self { permissions }
    }

    /// Rewrite `npm:package@version` to `https://esm.sh/package@version`.
    pub fn resolve_npm_specifier(specifier: &str) -> Option<String> {
        specifier
            .strip_prefix("npm:")
            .map(|rest| format!("https://esm.sh/{rest}"))
    }
}

impl ModuleLoader for WfeModuleLoader {
    fn resolve(
        &self,
        specifier: &str,
        referrer: &str,
        kind: ResolutionKind,
    ) -> Result<ModuleSpecifier, JsErrorBox> {
        // Block dynamic imports if not allowed.
        if matches!(kind, ResolutionKind::DynamicImport) {
            let checker = self.permissions.borrow();
            if !checker.allow_dynamic_import {
                return Err(JsErrorBox::generic(
                    "Dynamic import is not allowed by permissions",
                ));
            }
        }

        // ext: URLs are handled by deno_core.
        if specifier.starts_with("ext:") {
            return ModuleSpecifier::parse(specifier)
                .map_err(|e| JsErrorBox::generic(format!("Invalid ext URL: {e}")));
        }

        // npm: specifier -> rewrite to esm.sh URL.
        if let Some(esm_url) = Self::resolve_npm_specifier(specifier) {
            let parsed = ModuleSpecifier::parse(&esm_url)
                .map_err(|e| JsErrorBox::generic(format!("Invalid esm.sh URL: {e}")))?;
            return Ok(parsed);
        }

        // wfe: scheme for inline modules (used by load_main_es_module_from_code).
        if specifier.starts_with("wfe:") {
            return ModuleSpecifier::parse(specifier)
                .map_err(|e| JsErrorBox::generic(format!("Invalid wfe URL: {e}")));
        }

        // Absolute file:// URL.
        if specifier.starts_with("file://") {
            let parsed = ModuleSpecifier::parse(specifier)
                .map_err(|e| JsErrorBox::generic(format!("Invalid file URL: {e}")))?;
            let path = parsed
                .to_file_path()
                .map_err(|_| JsErrorBox::generic("Cannot convert URL to file path"))?;
            let checker = self.permissions.borrow();
            checker
                .check_read(
                    path.to_str()
                        .ok_or_else(|| JsErrorBox::generic("Non-UTF8 path"))?,
                )
                .map_err(|e| JsErrorBox::new("PermissionError", e.to_string()))?;
            return Ok(parsed);
        }

        // Absolute https:// URL.
        if specifier.starts_with("https://") || specifier.starts_with("http://") {
            let parsed = ModuleSpecifier::parse(specifier)
                .map_err(|e| JsErrorBox::generic(format!("Invalid URL: {e}")))?;
            let host = parsed
                .host_str()
                .ok_or_else(|| JsErrorBox::generic("URL has no host"))?;
            let checker = self.permissions.borrow();
            checker
                .check_net(host)
                .map_err(|e| JsErrorBox::new("PermissionError", e.to_string()))?;
            return Ok(parsed);
        }

        // Relative or bare path — resolve against referrer.
        // This handles ./foo, ../foo, and /foo (absolute path on same origin, e.g. esm.sh redirects)
        if specifier.starts_with("./") || specifier.starts_with("../") || specifier.starts_with('/')
        {
            let base = ModuleSpecifier::parse(referrer)
                .map_err(|e| JsErrorBox::generic(format!("Invalid referrer '{referrer}': {e}")))?;
            let resolved = base.join(specifier).map_err(|e| {
                JsErrorBox::generic(format!("Failed to resolve '{specifier}': {e}"))
            })?;

            // Check permissions based on scheme.
            match resolved.scheme() {
                "file" => {
                    let path = resolved
                        .to_file_path()
                        .map_err(|_| JsErrorBox::generic("Cannot convert URL to file path"))?;
                    let checker = self.permissions.borrow();
                    checker
                        .check_read(
                            path.to_str()
                                .ok_or_else(|| JsErrorBox::generic("Non-UTF8 path"))?,
                        )
                        .map_err(|e| JsErrorBox::new("PermissionError", e.to_string()))?;
                }
                "https" | "http" => {
                    let host = resolved
                        .host_str()
                        .ok_or_else(|| JsErrorBox::generic("URL has no host"))?;
                    let checker = self.permissions.borrow();
                    checker
                        .check_net(host)
                        .map_err(|e| JsErrorBox::new("PermissionError", e.to_string()))?;
                }
                _ => {}
            }

            return Ok(resolved);
        }

        Err(JsErrorBox::generic(format!(
            "Unsupported module specifier: '{specifier}'"
        )))
    }

    fn load(
        &self,
        module_specifier: &ModuleSpecifier,
        _maybe_referrer: Option<&ModuleLoadReferrer>,
        _options: ModuleLoadOptions,
    ) -> ModuleLoadResponse {
        let specifier = module_specifier.clone();
        let permissions = self.permissions.clone();

        match specifier.scheme() {
            "ext" => {
                // Handled by deno_core; should not reach here.
                ModuleLoadResponse::Sync(Err(JsErrorBox::generic(
                    "ext: modules are handled by deno_core",
                )))
            }
            "https" | "http" => {
                // Fetch over network.
                let url = specifier.to_string();
                ModuleLoadResponse::Async(Box::pin(async move {
                    // Permission check on the host.
                    let host = specifier
                        .host_str()
                        .ok_or_else(|| JsErrorBox::generic("URL has no host"))?
                        .to_string();
                    {
                        let checker = permissions.borrow();
                        checker
                            .check_net(&host)
                            .map_err(|e| JsErrorBox::new("PermissionError", e.to_string()))?;
                    }

                    let response = reqwest::get(&url).await.map_err(|e| {
                        JsErrorBox::generic(format!("Failed to fetch module '{url}': {e}"))
                    })?;

                    if !response.status().is_success() {
                        return Err(JsErrorBox::generic(format!(
                            "Failed to fetch module '{url}': HTTP {}",
                            response.status()
                        )));
                    }

                    let code: String = response.text().await.map_err(|e| {
                        JsErrorBox::generic(format!("Failed to read module body '{url}': {e}"))
                    })?;

                    Ok(ModuleSource::new(
                        ModuleType::JavaScript,
                        ModuleSourceCode::String(code.into()),
                        &specifier,
                        None,
                    ))
                }))
            }
            "file" => {
                // Read from disk.
                let path_result = specifier
                    .to_file_path()
                    .map_err(|_| JsErrorBox::generic("Cannot convert URL to file path"));
                match path_result {
                    Ok(path) => {
                        let path_str = path.to_string_lossy().to_string();

                        // Permission check.
                        {
                            let checker = permissions.borrow();
                            if let Err(e) = checker.check_read(&path_str) {
                                return ModuleLoadResponse::Sync(Err(JsErrorBox::new(
                                    "PermissionError",
                                    e.to_string(),
                                )));
                            }
                        }

                        match std::fs::read_to_string(&path) {
                            Ok(code) => ModuleLoadResponse::Sync(Ok(ModuleSource::new(
                                ModuleType::JavaScript,
                                ModuleSourceCode::String(code.into()),
                                &specifier,
                                None,
                            ))),
                            Err(e) => ModuleLoadResponse::Sync(Err(JsErrorBox::generic(format!(
                                "Failed to read module '{}': {e}",
                                path.display()
                            )))),
                        }
                    }
                    Err(e) => ModuleLoadResponse::Sync(Err(e)),
                }
            }
            scheme => ModuleLoadResponse::Sync(Err(JsErrorBox::generic(format!(
                "Unsupported module scheme: '{scheme}'"
            )))),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::executors::deno::config::DenoPermissions;

    fn make_loader(perms: DenoPermissions) -> WfeModuleLoader {
        let checker = PermissionChecker::from_config(&perms);
        WfeModuleLoader::new(Rc::new(RefCell::new(checker)))
    }

    #[test]
    fn resolve_npm_to_esm_sh() {
        let result = WfeModuleLoader::resolve_npm_specifier("npm:lodash@4.17.21");
        assert_eq!(result, Some("https://esm.sh/lodash@4.17.21".to_string()));
    }

    #[test]
    fn resolve_npm_scoped_package() {
        let result = WfeModuleLoader::resolve_npm_specifier("npm:@org/pkg@1.0.0");
        assert_eq!(result, Some("https://esm.sh/@org/pkg@1.0.0".to_string()));
    }

    #[test]
    fn resolve_non_npm_returns_none() {
        assert!(WfeModuleLoader::resolve_npm_specifier("https://example.com/mod.js").is_none());
        assert!(WfeModuleLoader::resolve_npm_specifier("./local.js").is_none());
    }

    #[test]
    fn resolve_npm_specifier_via_loader() {
        let loader = make_loader(DenoPermissions {
            net: vec!["esm.sh".to_string()],
            ..Default::default()
        });
        let result = loader
            .resolve(
                "npm:lodash@4",
                "ext:wfe/bootstrap.js",
                ResolutionKind::Import,
            )
            .unwrap();
        assert_eq!(result.as_str(), "https://esm.sh/lodash@4");
    }

    #[test]
    fn resolve_https_with_permission() {
        let loader = make_loader(DenoPermissions {
            net: vec!["cdn.example.com".to_string()],
            ..Default::default()
        });
        let result = loader
            .resolve(
                "https://cdn.example.com/mod.js",
                "ext:wfe/bootstrap.js",
                ResolutionKind::Import,
            )
            .unwrap();
        assert_eq!(result.as_str(), "https://cdn.example.com/mod.js");
    }

    #[test]
    fn resolve_https_without_permission_fails() {
        let loader = make_loader(DenoPermissions::default());
        let result = loader.resolve(
            "https://evil.com/mod.js",
            "ext:wfe/bootstrap.js",
            ResolutionKind::Import,
        );
        assert!(result.is_err());
        assert!(
            result
                .unwrap_err()
                .to_string()
                .contains("Permission denied")
        );
    }

    #[test]
    fn resolve_dynamic_import_denied() {
        let loader = make_loader(DenoPermissions {
            net: vec!["cdn.example.com".to_string()],
            dynamic_import: false,
            ..Default::default()
        });
        let result = loader.resolve(
            "https://cdn.example.com/mod.js",
            "ext:wfe/bootstrap.js",
            ResolutionKind::DynamicImport,
        );
        assert!(result.is_err());
        assert!(
            result
                .unwrap_err()
                .to_string()
                .contains("Dynamic import is not allowed")
        );
    }

    #[test]
    fn resolve_dynamic_import_allowed() {
        let loader = make_loader(DenoPermissions {
            net: vec!["cdn.example.com".to_string()],
            dynamic_import: true,
            ..Default::default()
        });
        let result = loader.resolve(
            "https://cdn.example.com/mod.js",
            "ext:wfe/bootstrap.js",
            ResolutionKind::DynamicImport,
        );
        assert!(result.is_ok());
    }

    #[test]
    fn resolve_ext_url() {
        let loader = make_loader(DenoPermissions::default());
        let result = loader
            .resolve(
                "ext:wfe/bootstrap.js",
                "ext:wfe/bootstrap.js",
                ResolutionKind::Import,
            )
            .unwrap();
        assert_eq!(result.as_str(), "ext:wfe/bootstrap.js");
    }

    #[test]
    fn resolve_relative_from_file_referrer() {
        let loader = make_loader(DenoPermissions {
            read: vec!["/tmp".to_string()],
            ..Default::default()
        });
        let result = loader
            .resolve("./helper.js", "file:///tmp/main.js", ResolutionKind::Import)
            .unwrap();
        assert_eq!(result.as_str(), "file:///tmp/helper.js");
    }

    #[test]
    fn resolve_relative_without_read_permission_fails() {
        let loader = make_loader(DenoPermissions::default());
        let result = loader.resolve("./helper.js", "file:///tmp/main.js", ResolutionKind::Import);
        assert!(result.is_err());
    }

    #[test]
    fn resolve_unsupported_specifier() {
        let loader = make_loader(DenoPermissions::default());
        let result = loader.resolve(
            "ftp://bad.com/mod.js",
            "ext:wfe/bootstrap.js",
            ResolutionKind::Import,
        );
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("Unsupported"));
    }
}
