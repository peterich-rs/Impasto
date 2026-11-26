//! Compares the standard library declarations with the public QuickJS surface.
#![expect(
    clippy::unwrap_used,
    reason = "test fixtures and diagnostics must be present"
)]

use std::path::Path;

use javascript::{runtime::Runtime, standard::StandardProfile};
use oxc::{
    allocator::Allocator,
    ast::ast::{
        BindingPattern, ClassElement, MethodDefinitionKind, PropertyKey, Statement,
        TSAccessibility, TSSignature, TSType,
    },
    codegen::Codegen,
    parser::Parser,
    semantic::SemanticBuilder,
    span::{GetSpan, SourceType},
    transformer::{TransformOptions, Transformer},
};
use rquickjs::CatchResultExt;
use serde_json::{Map, Value, json};

#[test]
fn standard_runtime_matches_every_declared_global_and_member() {
    let profile = StandardProfile::new();
    let declarations = profile.declarations();
    let expected = declared_surface(&declarations);
    let baseline = Runtime::builder().build_unstarted().unwrap();
    let keys = evaluate(&baseline, "JSON.stringify(Reflect.ownKeys(globalThis))");
    drop(baseline);
    let runtime = Runtime::builder().standard(profile).build().unwrap();
    let inspect = include_str!("fixtures/standard-surface.js");
    let fixture = fixture_js();
    let installed = evaluate(
        &runtime,
        &format!(
            "(function () {{ {inspect}\n const installed = inspectSurface({keys}, {expected});\n{fixture}\nreturn JSON.stringify(installed); }})()"
        ),
    );
    let mut installed: Value = serde_json::from_str(&installed).unwrap();
    let names = expected.as_object().unwrap();
    for (name, shape) in names {
        let Some(members) = shape["prototype"].as_object() else {
            continue;
        };
        for key in members.keys() {
            let mut parent = shape["parent"].as_str();
            while let Some(class) = parent {
                if !installed[name]["prototype"][key].is_null() {
                    break;
                }
                if let Some(value) = installed[class]["prototype"].get(key).cloned() {
                    installed[name]["prototype"][key] = value;
                    break;
                }
                parent = names[class]["parent"].as_str();
            }
        }
    }
    assert_eq!(installed, expected);
}

fn evaluate(runtime: &Runtime, source: &str) -> String {
    let extension = runtime.extension();
    let _budget = extension.budget("standard contract", 0);
    extension.with(|ctx| ctx.eval::<String, _>(source).catch(&ctx).unwrap())
}

fn fixture_js() -> String {
    let source = include_str!("fixtures/standard.ts");
    let allocator = Allocator::default();
    let parsed = Parser::new(&allocator, source, SourceType::ts()).parse();
    assert!(parsed.diagnostics.is_empty(), "{:?}", parsed.diagnostics);
    let mut program = parsed.program;
    let semantic = SemanticBuilder::new().build(&program);
    let result = Transformer::new(
        &allocator,
        Path::new("standard.ts"),
        &TransformOptions::default(),
    )
    .build_with_scoping(semantic.semantic.into_scoping(), &mut program);
    assert!(result.diagnostics.is_empty(), "{:?}", result.diagnostics);
    Codegen::new().build(&program).code
}

fn key(name: &PropertyKey<'_>, source: &str) -> String {
    if let Some(name) = name.static_name() {
        return name.into_owned();
    }
    let property = name.span().source_text(source);
    assert!(
        property.starts_with("Symbol."),
        "unsupported key {property}"
    );
    format!("[{property}]")
}

fn declared_surface(source: &str) -> Value {
    let allocator = Allocator::default();
    let parsed = Parser::new(&allocator, source, SourceType::d_ts()).parse();
    assert!(parsed.diagnostics.is_empty(), "{:?}", parsed.diagnostics);
    let mut out = Map::new();
    for statement in &parsed.program.body {
        match statement {
            Statement::ClassDeclaration(class) => {
                let mut statics = Map::new();
                let mut prototype = Map::new();
                let mut constructible = true;
                for member in &class.body.body {
                    let (name, is_static, visibility, kind) = match member {
                        ClassElement::MethodDefinition(member) => {
                            if member.kind == MethodDefinitionKind::Constructor {
                                constructible =
                                    member.accessibility != Some(TSAccessibility::Private);
                                continue;
                            }
                            let kind = if member.kind == MethodDefinitionKind::Method {
                                "method"
                            } else {
                                "property"
                            };
                            (&member.key, member.r#static, member.accessibility, kind)
                        }
                        ClassElement::PropertyDefinition(member) => (
                            &member.key,
                            member.r#static,
                            member.accessibility,
                            "property",
                        ),
                        other => panic!("unsupported class member {other:?}"),
                    };
                    if visibility != Some(TSAccessibility::Private) {
                        let members = if is_static {
                            &mut statics
                        } else {
                            &mut prototype
                        };
                        members.insert(key(name, source), json!(kind));
                    }
                }
                out.insert(class.id.as_ref().unwrap().name.to_string(), json!({
                    "kind": "class",
                    "constructible": constructible,
                    "static": statics,
                    "prototype": prototype,
                    "parent": class.heritage.as_ref().map(|parent| parent.expression.span().source_text(source)),
                }));
            }
            Statement::FunctionDeclaration(function) => {
                out.insert(
                    function.id.as_ref().unwrap().name.to_string(),
                    json!({"kind": "function"}),
                );
            }
            Statement::VariableDeclaration(variable) => {
                for declaration in &variable.declarations {
                    let BindingPattern::BindingIdentifier(id) = &declaration.id else {
                        panic!("expected a global identifier");
                    };
                    let TSType::TSTypeLiteral(ty) = &declaration
                        .type_annotation
                        .as_ref()
                        .unwrap()
                        .type_annotation
                    else {
                        panic!("expected a global object type");
                    };
                    let own: Map<String, Value> = ty
                        .members
                        .iter()
                        .map(|member| {
                            let (name, kind) = match member {
                                TSSignature::TSMethodSignature(method) => (&method.key, "method"),
                                TSSignature::TSPropertySignature(property) => {
                                    (&property.key, "property")
                                }
                                other => panic!("unsupported object member {other:?}"),
                            };
                            (key(name, source), json!(kind))
                        })
                        .collect();
                    out.insert(id.name.to_string(), json!({"kind": "object", "own": own}));
                }
            }
            Statement::TSInterfaceDeclaration(_) | Statement::TSTypeAliasDeclaration(_) => {}
            other => panic!("unsupported declaration {other:?}"),
        }
    }
    Value::Object(out)
}
