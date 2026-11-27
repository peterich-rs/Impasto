//! Merges platform projects and validates native permission mappings.

use std::collections::BTreeSet;
use std::path::Path;

use anyhow::{Context, Result, bail};

use super::{
    Descriptor, NativeProject, PermissionMethod, Platform, PlatformProject, under,
    valid_package_name,
};

impl Platform {
    pub(super) fn permissions_for(
        &self,
        linked: &bundler::Bundle,
        dependencies: &BTreeSet<String>,
        retained: &BTreeSet<String>,
    ) -> BTreeSet<String> {
        self.permission_rules
            .iter()
            .filter(|(owner, _)| retained.contains(*owner))
            .flat_map(|(owner, rules)| {
                rules.iter().filter_map(move |(permission, methods)| {
                    let required = dependencies.contains(owner)
                        || methods.is_empty()
                        || methods.iter().any(|method| {
                            linked
                                .native_use
                                .get(&method.specifier)
                                .is_some_and(|names| {
                                    names.as_ref().is_none_or(|names| {
                                        names.contains("default")
                                            || names.contains(&method.export)
                                            || method
                                                .object_export
                                                .as_ref()
                                                .is_some_and(|name| names.contains(name))
                                    })
                                })
                        });
                    required.then(|| permission.clone())
                })
            })
            .collect()
    }

    pub(super) fn resolve_permission_methods(&mut self, modules: &[Descriptor]) -> Result<()> {
        for (owner, rules) in &mut self.permission_rules {
            for (permission, methods) in rules {
                for method in methods {
                    let module = modules
                        .iter()
                        .find(|module| {
                            module.owner == *owner
                                && module.specifier == method.specifier
                                && module.exports.contains(&method.export)
                        })
                        .with_context(|| {
                            format!(
                                "{owner}: {permission} 引用了未声明的原生导出 {}#{}",
                                method.specifier, method.export
                            )
                        })?;
                    method.object_export = module
                        .exports
                        .contains(&module.name)
                        .then(|| module.name.clone());
                }
            }
        }
        Ok(())
    }

    pub(super) fn add_project(
        &mut self,
        project: PlatformProject,
        root: &Path,
        target: &str,
        cargo_name: &str,
    ) -> Result<()> {
        for (permission, methods) in project.permissions {
            if target != "android"
                || !permission.contains('.')
                || !permission.split('.').all(|part| {
                    !part.is_empty()
                        && part
                            .bytes()
                            .all(|byte| byte.is_ascii_alphanumeric() || byte == b'_')
                })
            {
                bail!("无效的 Android 权限 {permission}");
            }
            let methods = methods
                .into_iter()
                .map(|method| {
                    let (specifier, export) = method.rsplit_once('#').with_context(|| {
                        format!("{permission} 的方法需要使用 module#export 格式")
                    })?;
                    Ok(PermissionMethod {
                        specifier: specifier.into(),
                        export: export.into(),
                        object_export: None,
                    })
                })
                .collect::<Result<Vec<_>>>()?;
            self.android_permissions.insert(permission.clone());
            self.permission_rules
                .entry(cargo_name.into())
                .or_default()
                .insert(permission, methods);
        }
        self.usage_description_keys
            .extend(project.usage_description_keys);
        self.required_entitlements
            .extend(project.required_entitlements);
        let Some(path) = project.path else {
            if project.package.is_some()
                || project.module.is_some()
                || project.plugin_class.is_some()
            {
                bail!("平台插件的注册信息需要 path");
            }
            return Ok(());
        };
        let path = under(root, &path)?;
        let package = project.package.context("平台插件需要声明 package")?;
        let plugin_class = project.plugin_class;
        let identifier = |value: &str| {
            value.starts_with(|ch: char| ch.is_ascii_alphabetic() || ch == '_')
                && value
                    .bytes()
                    .all(|ch| ch.is_ascii_alphanumeric() || ch == b'_')
        };
        let module = project.module.unwrap_or_else(|| package.clone());
        if plugin_class
            .as_deref()
            .is_some_and(|name| !identifier(name))
            || (target != "android" && plugin_class.is_none())
            || (target == "android" && !package.split('.').all(identifier))
            || (target != "android" && (!identifier(&module) || !valid_package_name(&package)))
        {
            bail!("平台插件包、模块或注册类名称无效");
        }
        let required = if target == "android" {
            "build.gradle.kts"
        } else {
            "Package.swift"
        };
        if !path.join(required).is_file() {
            bail!("缺少 {}", path.join(required).display());
        }
        if self.projects.iter().any(|item| item.package == package) {
            bail!("重复的平台插件包 {package}");
        }
        let gradle_module = cargo_name.replace('-', "_");
        if target == "android"
            && self
                .projects
                .iter()
                .any(|project| project.gradle_module == gradle_module)
        {
            bail!("重复的 Gradle 模块 {gradle_module}");
        }
        self.projects.push(NativeProject {
            path,
            package,
            gradle_module,
            module,
            plugin_class,
        });
        Ok(())
    }
}
