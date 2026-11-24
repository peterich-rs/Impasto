//! Expansions for the Impasto Rust SDK.

mod exports;
mod types;

use proc_macro::TokenStream;
use proc_macro2::TokenStream as Tokens;
use syn::{Error, Result, Type, parse_macro_input};

#[proc_macro_derive(ImpastoType, attributes(impasto))]
pub fn impasto_type(input: TokenStream) -> TokenStream {
    expand(types::derive_type(parse_macro_input!(input)))
}

#[proc_macro_derive(ImpastoEvents, attributes(impasto))]
pub fn impasto_events(input: TokenStream) -> TokenStream {
    expand(types::derive_events(parse_macro_input!(input)))
}

#[proc_macro_attribute]
pub fn module(args: TokenStream, input: TokenStream) -> TokenStream {
    expand(exports::module(args.into(), parse_macro_input!(input)))
}

#[proc_macro_attribute]
pub fn export(args: TokenStream, input: TokenStream) -> TokenStream {
    let result = local(args);
    let item = parse_macro_input!(input);
    expand(result.and_then(|local| exports::export(item, local)))
}

#[proc_macro_attribute]
pub fn object(args: TokenStream, input: TokenStream) -> TokenStream {
    let result = local(args);
    let item = parse_macro_input!(input);
    expand(result.and_then(|local| exports::object(item, local)))
}

fn local(args: TokenStream) -> Result<bool> {
    if args.is_empty() {
        return Ok(false);
    }
    let option: syn::Ident = syn::parse(args)?;
    if option == "local" {
        Ok(true)
    } else {
        Err(Error::new(option.span(), "expected local"))
    }
}

#[proc_macro_attribute]
pub fn method(_args: TokenStream, input: TokenStream) -> TokenStream {
    let mut output: TokenStream = Error::new(
        proc_macro2::Span::call_site(),
        "method options require an #[impasto::export] impl",
    )
    .into_compile_error()
    .into();
    output.extend(input);
    output
}

fn expand(result: Result<Tokens>) -> TokenStream {
    result.unwrap_or_else(Error::into_compile_error).into()
}

fn type_named(ty: &Type, name: &str) -> bool {
    matches!(ty, Type::Path(path) if path.path.segments.last().is_some_and(|segment| segment.ident == name))
}

fn type_arguments<'a>(ty: &'a Type, name: &str) -> Option<Vec<&'a Type>> {
    let Type::Path(path) = ty else { return None };
    let segment = path
        .path
        .segments
        .last()
        .filter(|segment| segment.ident == name)?;
    let syn::PathArguments::AngleBracketed(args) = &segment.arguments else {
        return None;
    };
    args.args
        .iter()
        .map(|arg| match arg {
            syn::GenericArgument::Type(ty) => Some(ty),
            _ => None,
        })
        .collect()
}
