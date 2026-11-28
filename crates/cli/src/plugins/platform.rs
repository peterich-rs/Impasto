//! Merges platform projects and validates native permission mappings.

use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;

use anyhow::{Context, Result, bail};

use super::{
    AndroidPermissionAttributes, Descriptor, NativeProject, PermissionMethod, Platform,
    PlatformProject, SwiftFeature, under, valid_package_name,
};
use crate::app::{App, PluginOptions};

impl Platform {
    pub(super) fn resolve_permission_attributes(&mut self) -> Result<()> {
        use std::collections::btree_map::Entry;

        let mut attributes = BTreeMap::new();
        for (permission, rule) in self.permission_attribute_rules.values().flatten() {
            if !self.android_permissions.contains(permission) {
                continue;
            }
            match attributes.entry(permission.clone()) {
                Entry::Vacant(entry) => {
                    entry.insert(rule.clone());
                }
                Entry::Occupied(entry) if entry.get() != rule => {
                    bail!("Android 权限 {permission} 的声明属性冲突");
                }
                Entry::Occupied(_) => {}
            }
        }
        self.android_permission_attributes = attributes;
        Ok(())
    }

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
        mut project: PlatformProject,
        root: &Path,
        target: &str,
        cargo_name: &str,
        options: Option<&PluginOptions>,
        features: Option<BTreeSet<String>>,
    ) -> Result<()> {
        let mut background_modes = BTreeSet::new();
        let mut swift_features = BTreeMap::new();
        for feature in features.iter().flatten() {
            let config = &project.features[feature];
            if !config.background_modes.is_empty()
                && (target != "ios" || config.background_modes.iter().any(|mode| !identifier(mode)))
            {
                bail!("无效的 iOS 后台模式 {feature}");
            }
            background_modes.extend(config.background_modes.iter().cloned());
            project
                .usage_description_keys
                .extend(config.usage_description_keys.clone());
            project
                .required_entitlements
                .extend(config.required_entitlements.clone());
            if target != "android" {
                let product = config.product.as_ref().context("Swift 特性缺少 product")?;
                let module = config.module.as_ref().unwrap_or(product);
                let registrar = config
                    .registrar
                    .as_ref()
                    .context("Swift 特性缺少 registrar")?;
                if !valid_package_name(product) || !identifier(module) || !identifier(registrar) {
                    bail!("无效的 Swift 特性 {feature}");
                }
                swift_features.insert(
                    (product.to_owned(), module.to_owned(), registrar.to_owned()),
                    SwiftFeature {
                        product: product.to_owned(),
                        module: module.to_owned(),
                        registrar: registrar.to_owned(),
                    },
                );
            }
        }
        for (permission, rule) in project.permission_rules {
            if target != "android" || !valid_permission(&permission) {
                bail!("无效的 Android 权限 {permission}");
            }
            if rule.max_sdk == Some(0)
                || rule
                    .max_sdk_by_feature
                    .values()
                    .any(|maximum| *maximum == 0)
                || rule
                    .uses_permission_flags
                    .iter()
                    .any(|flag| flag != "neverForLocation")
            {
                bail!("无效的 Android 权限属性 {permission}");
            }
            if rule
                .feature
                .iter()
                .chain(&rule.features)
                .chain(rule.max_sdk_by_feature.keys())
                .any(|name| !project.features.contains_key(name))
            {
                bail!("Android 权限 {permission} 引用未知特性");
            }
            let selected = rule
                .feature
                .iter()
                .chain(&rule.features)
                .filter(|name| {
                    features
                        .as_ref()
                        .is_none_or(|features| features.contains(*name))
                })
                .collect::<Vec<_>>();
            if selected.is_empty() && (rule.feature.is_some() || !rule.features.is_empty()) {
                continue;
            }
            project.permissions.entry(permission.clone()).or_default();
            let attributes = AndroidPermissionAttributes {
                max_sdk: rule.max_sdk.or_else(|| {
                    selected
                        .into_iter()
                        .map(|name| rule.max_sdk_by_feature.get(name).copied())
                        .collect::<Option<Vec<_>>>()?
                        .into_iter()
                        .max()
                }),
                uses_permission_flags: if options
                    .is_some_and(|options| options.android_bluetooth_never_for_location)
                {
                    rule.uses_permission_flags
                } else {
                    BTreeSet::new()
                },
            };
            if attributes != AndroidPermissionAttributes::default() {
                self.permission_attribute_rules
                    .entry(cargo_name.into())
                    .or_default()
                    .insert(permission, attributes);
            }
        }
        for (permission, methods) in project.permissions {
            if target != "android" || !valid_permission(&permission) {
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
                || project.prepare.is_some()
            {
                bail!("平台插件的注册信息需要 path");
            }
            return Ok(());
        };
        let path = under(root, &path)?;
        let package = project.package.context("平台插件需要声明 package")?;
        let plugin_class = project.plugin_class;
        if project
            .prepare
            .as_deref()
            .is_some_and(|method| target != "ios" || !identifier(method))
        {
            bail!("无效的 iOS 启动方法");
        }
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
            prepare: project.prepare,
            background_modes,
            features,
            swift_features: swift_features.into_values().collect(),
        });
        Ok(())
    }
}

impl PlatformProject {
    pub(super) fn select_features(
        &self,
        target: &str,
        app: &App,
        options: Option<&PluginOptions>,
    ) -> Result<Option<BTreeSet<String>>> {
        if self.features.is_empty() {
            return Ok(None);
        }
        if let Some(features) = options.and_then(|options| options.enabled.as_ref()) {
            return Ok(Some(
                features
                    .iter()
                    .filter(|name| self.features.contains_key(*name))
                    .cloned()
                    .collect(),
            ));
        }
        let mut selected = BTreeSet::new();
        for (name, feature) in &self.features {
            let (usage, entitlements) = match target {
                "ios" => (&app.ios.usage_descriptions, &app.ios.entitlements),
                "macos" => (&app.macos.usage_descriptions, &app.macos.entitlements),
                "android" => {
                    if feature.default
                        || (!feature.infer_permissions.is_empty()
                            && feature
                                .infer_permissions
                                .iter()
                                .all(|name| app.android.permissions.contains(name)))
                    {
                        selected.insert(name.clone());
                    }
                    continue;
                }
                _ => bail!("未知平台 {target}"),
            };
            if feature.default
                || (!feature.usage_description_keys.is_empty()
                    && feature
                        .usage_description_keys
                        .iter()
                        .all(|key| usage.contains_key(key))
                    && feature
                        .required_entitlements
                        .iter()
                        .all(|key| entitlements.get(key) == Some(&true)))
            {
                selected.insert(name.clone());
            }
        }
        Ok(Some(selected))
    }
}

fn identifier(value: &str) -> bool {
    value.starts_with(|ch: char| ch.is_ascii_alphabetic() || ch == '_')
        && value
            .bytes()
            .all(|ch| ch.is_ascii_alphanumeric() || ch == b'_')
}

fn valid_permission(value: &str) -> bool {
    value.contains('.')
        && value.split('.').all(|part| {
            !part.is_empty()
                && part
                    .bytes()
                    .all(|byte| byte.is_ascii_alphanumeric() || byte == b'_')
        })
}
