//! Manifest generation for converted projects (M06 §5): the author never
//! writes `poid.json` by hand — the converter derives a valid, maximally
//! restrictive manifest from what it saw.

use poid_core::{FilesystemAccess, Manifest, Permissions, ToolchainRecord};

use crate::infer::InferredPermissions;

/// Everything [`converted_manifest`] needs to know about one conversion.
#[derive(Debug, Clone)]
pub struct ConvertPlan {
    /// Human-readable application name (derived from a filename or folder).
    pub name: String,
    /// Reverse-DNS id; converted projects live under `local.poid.<slug>`
    /// until the author claims a real one.
    pub app_id: String,
    /// Inferred permissions (network is never in here — deny by default).
    pub inferred: InferredPermissions,
    /// `name@version` records for `runtime.bundled_deps` (Standard Library
    /// selections and Resolver downloads).
    pub bundled_deps: Vec<String>,
    /// Builder identity for `runtime.toolchain.builder`.
    pub builder: String,
    /// Exact esbuild version used, when a build happened.
    pub esbuild: Option<String>,
}

/// Derives a `local.poid.<slug>` id component from a display name.
pub fn slug_of(name: &str) -> String {
    let mut slug: String = name
        .to_lowercase()
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() { c } else { '-' })
        .collect();
    while slug.contains("--") {
        slug = slug.replace("--", "-");
    }
    let trimmed = slug.trim_matches('-');
    if trimmed.is_empty() {
        "app".to_owned()
    } else {
        trimmed.to_owned()
    }
}

/// Builds the manifest for a converted project: `type: app`, `web` profile,
/// entry `app/index.html`, embedded storage, inferred permissions, recorded
/// toolchain. Validates before returning — a converter that emits an invalid
/// manifest is a bug, not a user error.
pub fn converted_manifest(plan: &ConvertPlan) -> Result<Manifest, poid_core::ManifestError> {
    let mut manifest = Manifest::new_app(
        plan.app_id.clone(),
        plan.name.clone(),
        "1.0.0",
        "app/index.html",
    );

    manifest.permissions = Some(Permissions {
        network: Some(Vec::new()),
        filesystem: Some(if plan.inferred.filesystem_user_initiated {
            FilesystemAccess::UserInitiated
        } else {
            FilesystemAccess::None
        }),
        clipboard: Some(plan.inferred.clipboard),
        print: Some(plan.inferred.print),
        notifications: Some(plan.inferred.notifications),
        mcp: Some(Vec::new()),
        extra: Default::default(),
    });

    if let Some(runtime) = &mut manifest.runtime {
        if !plan.bundled_deps.is_empty() {
            let mut deps = plan.bundled_deps.clone();
            deps.sort();
            runtime.bundled_deps = Some(deps);
        }
        runtime.toolchain = Some(ToolchainRecord {
            builder: Some(plan.builder.clone()),
            esbuild: plan.esbuild.clone(),
            extra: Default::default(),
        });
    }

    manifest.validate()?;
    Ok(manifest)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn slugs_are_reverse_dns_safe() {
        assert_eq!(slug_of("My Cool App!"), "my-cool-app");
        assert_eq!(slug_of("---"), "app");
        assert_eq!(slug_of("Wykres Przepływów"), "wykres-przep-yw-w");
    }

    #[test]
    fn converted_manifests_validate_and_stay_restrictive() {
        let plan = ConvertPlan {
            name: "Kanban".into(),
            app_id: format!("local.poid.{}", slug_of("Kanban")),
            inferred: InferredPermissions {
                clipboard: true,
                ..Default::default()
            },
            bundled_deps: vec!["react@18.3.1".into(), "lodash-es@4.17.21".into()],
            builder: "poid-cli@0.0.1".into(),
            esbuild: Some("0.25.12".into()),
        };
        let manifest = converted_manifest(&plan).expect("valid");
        let perms = manifest.permissions.as_ref().expect("perms");
        assert_eq!(perms.network.as_deref(), Some(&[][..]));
        assert_eq!(perms.clipboard, Some(true));
        assert_eq!(perms.print, Some(false));
        let runtime = manifest.runtime.as_ref().expect("runtime");
        assert_eq!(
            runtime.bundled_deps.as_ref().map(|d| d.len()),
            Some(2),
            "stdlib selections are recorded"
        );
    }
}
