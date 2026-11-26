use std::collections::BTreeSet;

use heck::{ToKebabCase, ToLowerCamelCase, ToSnakeCase};
use proc_macro2::TokenStream;
use quote::{format_ident, quote};
use syn::{Attribute, Data, DeriveInput, Error, Fields, Ident, LitStr, Result, Type};

use crate::{type_arguments, type_named};

#[derive(Default)]
struct Names {
    rename: Option<String>,
    rename_all: Option<String>,
    tag: Option<String>,
    nullable: bool,
    as_string: bool,
    bytes: bool,
}

impl Names {
    fn parse(attrs: &[Attribute]) -> Result<Self> {
        let mut options = Self::default();
        for attr in attrs.iter().filter(|attr| attr.path().is_ident("impasto")) {
            attr.parse_nested_meta(|meta| {
                if meta.path.is_ident("rename") {
                    options.rename = Some(meta.value()?.parse::<LitStr>()?.value());
                } else if meta.path.is_ident("rename_all") {
                    let value = meta.value()?.parse::<LitStr>()?;
                    if !matches!(
                        value.value().as_str(),
                        "camelCase" | "snake_case" | "kebab-case"
                    ) {
                        return Err(Error::new(value.span(), "unsupported naming style"));
                    }
                    options.rename_all = Some(value.value());
                } else if meta.path.is_ident("tag") {
                    options.tag = Some(meta.value()?.parse::<LitStr>()?.value());
                } else if meta.path.is_ident("nullable") {
                    options.nullable = true;
                } else if meta.path.is_ident("as_string") {
                    options.as_string = true;
                } else if meta.path.is_ident("bytes") {
                    options.bytes = true;
                } else {
                    return Err(meta.error("unknown Impasto type option"));
                }
                Ok(())
            })?;
        }
        Ok(options)
    }
}

fn name(ident: &Ident, options: &Names, parent: &Names) -> String {
    if let Some(rename) = &options.rename {
        return rename.clone();
    }
    let source = ident.to_string();
    match parent.rename_all.as_deref() {
        Some("snake_case") => source.to_snake_case(),
        Some("kebab-case") => source.to_kebab_case(),
        _ => source.to_lower_camel_case(),
    }
}

struct Field<'a> {
    ident: &'a Ident,
    ty: &'a Type,
    key: String,
    optional: bool,
    nullable: bool,
    representation: Representation,
}

enum Representation {
    Value,
    TextInteger,
    Bytes,
}

fn fields<'a>(
    fields: &'a Fields,
    naming: &Names,
    reserved: Option<&str>,
) -> Result<Vec<Field<'a>>> {
    let Fields::Named(fields) = fields else {
        if matches!(fields, Fields::Unit) {
            return Ok(Vec::new());
        }
        return Err(Error::new_spanned(
            fields,
            "Impasto DTO fields must be named",
        ));
    };
    let mut keys = BTreeSet::new();
    fields
        .named
        .iter()
        .map(|field| {
            let ident = field
                .ident
                .as_ref()
                .ok_or_else(|| Error::new_spanned(field, "expected named field"))?;
            let options = Names::parse(&field.attrs)?;
            let key = name(ident, &options, naming);
            if reserved == Some(key.as_str()) || !keys.insert(key.clone()) {
                return Err(Error::new_spanned(
                    ident,
                    format!("duplicate field {key:?}"),
                ));
            }

            let ty = &field.ty;
            let optional = type_arguments(ty, "Option").is_some();
            let representation = if options.as_string
                && !options.bytes
                && (type_named(ty, "u64") || type_named(ty, "i64"))
            {
                Representation::TextInteger
            } else if options.bytes
                && !options.as_string
                && type_arguments(ty, "Vec")
                    .is_some_and(|inner| inner.first().is_some_and(|item| type_named(item, "u8")))
            {
                Representation::Bytes
            } else if options.bytes || options.as_string {
                return Err(Error::new_spanned(
                    ty,
                    "as_string requires i64/u64; bytes requires Vec<u8>",
                ));
            } else {
                Representation::Value
            };

            Ok(Field {
                ident,
                key,
                ty,
                optional,
                nullable: options.nullable,
                representation,
            })
        })
        .collect()
}

impl Field<'_> {
    fn decode(&self) -> TokenStream {
        let Self {
            ident,
            key,
            ty,
            representation,
            ..
        } = self;
        let value = match representation {
            Representation::Value => {
                quote!(::impasto::macro_support::decode_field::<#ty>(&mut fields, #key)?)
            }
            Representation::TextInteger => quote!({
                let text = ::impasto::macro_support::decode_field::<String>(&mut fields, #key)?;
                text.parse::<#ty>().map_err(|error| format!("{}: {}", #key, error))?
            }),
            Representation::Bytes => {
                quote!(::impasto::macro_support::decode_field::<::impasto::Bytes>(&mut fields, #key)?.0)
            }
        };
        quote!(#ident: #value)
    }

    fn encode(&self, value: TokenStream) -> TokenStream {
        let Self {
            key,
            representation,
            optional,
            nullable,
            ..
        } = self;
        if *optional && !*nullable {
            return quote! {
                if let Some(value) = #value {
                    fields.push((#key.into(), ::impasto::IntoPluginValue::into_value(value)?));
                }
            };
        }
        let value = match representation {
            Representation::Value => value,
            Representation::TextInteger => quote!((#value).to_string()),
            Representation::Bytes => quote!(::impasto::Bytes(#value)),
        };
        quote!(fields.push((#key.into(), ::impasto::IntoPluginValue::into_value(#value)?));)
    }

    fn declaration(&self) -> TokenStream {
        let Self {
            key,
            ty,
            representation,
            optional,
            nullable,
            ..
        } = self;
        let mark = if *optional && !*nullable { "?" } else { "" };
        let name = ts_property(key);
        let ts = match representation {
            Representation::Value => quote!(<#ty as ::impasto::ImpastoType>::typescript(types)),
            Representation::TextInteger => quote!("string"),
            Representation::Bytes => quote!("Uint8Array | ArrayBuffer"),
        };
        quote!(format!("{}{}: {}", #name, #mark, #ts))
    }
}

fn ts_property(key: &str) -> String {
    let mut chars = key.chars();
    if chars
        .next()
        .is_some_and(|ch| ch.is_ascii_alphabetic() || matches!(ch, '_' | '$'))
        && chars.all(|ch| ch.is_ascii_alphanumeric() || matches!(ch, '_' | '$'))
    {
        key.into()
    } else {
        format!("{key:?}")
    }
}

struct Variant<'a> {
    ident: &'a Ident,
    key: String,
    fields: Vec<Field<'a>>,
    unit: bool,
}

fn variants<'a>(data: &'a syn::DataEnum, naming: &Names, tag: &str) -> Result<Vec<Variant<'a>>> {
    let mut names = BTreeSet::new();
    data.variants
        .iter()
        .map(|item| {
            let options = Names::parse(&item.attrs)?;
            let key = name(&item.ident, &options, naming);
            if !names.insert(key.clone()) {
                return Err(Error::new_spanned(
                    item,
                    format!("duplicate variant {key:?}"),
                ));
            }
            Ok(Variant {
                ident: &item.ident,
                key,
                fields: fields(&item.fields, &options, Some(tag))?,
                unit: matches!(item.fields, Fields::Unit),
            })
        })
        .collect()
}

fn capture_fields<'a>(fields: &'a [Field<'_>]) -> impl Iterator<Item = TokenStream> + 'a {
    fields.iter().enumerate().map(|(index, field)| {
        let rust = field.ident;
        let value = format_ident!("__impasto_field_{index}");
        quote!(#rust: #value)
    })
}

fn encode_fields<'a>(fields: &'a [Field<'_>]) -> impl Iterator<Item = TokenStream> + 'a {
    fields.iter().enumerate().map(|(index, field)| {
        let value = format_ident!("__impasto_field_{index}");
        field.encode(quote!(#value))
    })
}

pub fn derive_type(input: DeriveInput) -> Result<TokenStream> {
    if !input.generics.params.is_empty() {
        return Err(Error::new_spanned(
            input.generics,
            "Impasto DTOs require concrete types",
        ));
    }
    let ident = &input.ident;
    let naming = Names::parse(&input.attrs)?;
    let name = naming.rename.clone().unwrap_or_else(|| ident.to_string());
    let read_map = quote! {
        let ::impasto::abi::PluginValue::Map(items) = value else { return Err("expected an object".into()); };
        let mut fields: ::std::collections::BTreeMap<_, _> = items.into_iter().collect();
    };
    let (decode, encode, declaration) = match &input.data {
        Data::Struct(data) => {
            if !matches!(&data.fields, Fields::Named(_)) {
                return Err(Error::new_spanned(
                    ident,
                    "Impasto structs require named fields",
                ));
            }
            let fields = fields(&data.fields, &naming, None)?;
            let decode = fields.iter().map(Field::decode);
            let encode = fields.iter().map(|field| {
                let ident = field.ident;
                field.encode(quote!(self.#ident))
            });
            let declarations = fields.iter().map(Field::declaration);
            (
                quote!(#read_map Ok(Self { #(#decode),* })),
                quote! {
                    let mut fields = Vec::new();
                    #(#encode)*
                    Ok(::impasto::abi::PluginValue::Map(fields))
                },
                quote! {
                    let fields: Vec<String> = vec![#(#declarations),*];
                    format!("export interface {} {{\n  {}\n}}", #name, fields.join(";\n  "))
                },
            )
        }
        Data::Enum(data) => {
            let tag = naming.tag.as_deref().unwrap_or("type");
            let tag_property = ts_property(tag);
            let variants = variants(data, &naming, tag)?;
            if variants.is_empty() {
                return Err(Error::new_spanned(ident, "Impasto enums require a variant"));
            }
            let unit = variants.iter().all(|variant| variant.unit);
            let decodes = variants.iter().map(|variant| {
                let Variant {
                    ident,
                    key,
                    fields,
                    unit,
                } = variant;
                if *unit {
                    quote!(#key => Ok(Self::#ident))
                } else {
                    let fields = fields.iter().map(Field::decode);
                    quote!(#key => Ok(Self::#ident { #(#fields),* }))
                }
            });
            let encodes = variants.iter().map(|variant| {
                let Variant { ident, key, fields, unit: variant_unit } = variant;
                if unit {
                    quote!(Self::#ident => Ok(::impasto::abi::PluginValue::Str(#key.into())))
                } else {
                    let pattern = if *variant_unit {
                        quote!(Self::#ident)
                    } else {
                        let idents = capture_fields(fields);
                        quote!(Self::#ident { #(#idents),* })
                    };
                    let fields = encode_fields(fields);
                    quote!(#pattern => {
                        let mut fields = vec![(#tag.into(), ::impasto::abi::PluginValue::Str(#key.into()))];
                        #(#fields)*
                        Ok(::impasto::abi::PluginValue::Map(fields))
                    })
                }
            });
            let declarations = variants.iter().map(|variant| {
                let key = format!("{:?}", variant.key);
                if unit {
                    quote!(#key.to_owned())
                } else {
                    let declarations = variant.fields.iter().map(Field::declaration);
                    quote!({
                        let fields =
                            vec![format!("{}: {}", #tag_property, #key), #(#declarations),*];
                        format!("{{ {} }}", fields.join("; "))
                    })
                }
            });
            let discriminant = if unit {
                quote!(let kind = <String as ::impasto::FromPluginValue>::from_value(value)?;)
            } else {
                quote!(#read_map let kind = ::impasto::macro_support::decode_field::<String>(&mut fields, #tag)?;)
            };
            (
                quote!(#discriminant match kind.as_str() {
                    #(#decodes,)*
                    _ => Err(format!("unknown {} variant {:?}", #name, kind)),
                }),
                quote!(match self { #(#encodes,)* }),
                quote! {
                    let variants = vec![#(#declarations),*];
                    format!("export type {} = {};", #name, variants.join(" | "))
                },
            )
        }
        Data::Union(_) => return Err(Error::new_spanned(ident, "Impasto does not support unions")),
    };
    Ok(quote! {
        impl ::impasto::FromPluginValue for #ident {
            fn from_value(value: ::impasto::abi::PluginValue) -> Result<Self, String> { #decode }
        }
        impl ::impasto::IntoPluginValue for #ident {
            fn into_value(self) -> Result<::impasto::abi::PluginValue, String> { #encode }
        }
        impl ::impasto::ImpastoType for #ident {
            fn typescript(types: &mut ::impasto::TsTypes) -> String {
                types.define::<Self>(#name, |types| { let _ = &types; #declaration });
                #name.into()
            }
        }
    })
}

pub fn derive_events(input: DeriveInput) -> Result<TokenStream> {
    let Data::Enum(data) = &input.data else {
        return Err(Error::new_spanned(
            &input.ident,
            "Impasto events require an enum",
        ));
    };
    if !input.generics.params.is_empty() {
        return Err(Error::new_spanned(
            input.generics,
            "Impasto events require concrete types",
        ));
    }
    let ident = &input.ident;
    let naming = Names::parse(&input.attrs)?;
    let mut seen = BTreeSet::new();
    let mut metadata = Vec::new();
    let mut declarations = Vec::new();
    let mut encode = Vec::new();
    for variant in &data.variants {
        let ident = &variant.ident;
        let options = Names::parse(&variant.attrs)?;
        let key = name(ident, &options, &naming);
        if !seen.insert(key.clone()) {
            return Err(Error::new_spanned(
                ident,
                format!("duplicate event {key:?}"),
            ));
        }
        let payload = if let Fields::Unnamed(fields) = &variant.fields {
            let mut fields = fields.unnamed.iter();
            let Some(field) = fields.next().filter(|_| fields.next().is_none()) else {
                return Err(Error::new_spanned(
                    variant,
                    "an event can have only one positional field",
                ));
            };
            let ty = &field.ty;
            let value = quote!(<#ty as ::impasto::ImpastoType>::typescript(types));
            declarations.push(value.clone());
            encode.push(quote!(
                Self::#ident(value) => Ok((#key, ::impasto::IntoPluginValue::into_value(value)?))
            ));
            value
        } else {
            let fields = fields(&variant.fields, &naming, None)?;
            let types = fields.iter().map(Field::declaration).collect::<Vec<_>>();
            declarations.extend(types.iter().cloned());
            let pattern = if matches!(variant.fields, Fields::Unit) {
                quote!(Self::#ident)
            } else {
                let idents = capture_fields(&fields);
                quote!(Self::#ident { #(#idents),* })
            };
            let values = encode_fields(&fields);
            encode.push(quote!(#pattern => {
                let mut fields = Vec::new();
                #(#values)*
                Ok((#key, ::impasto::abi::PluginValue::Map(fields)))
            }));
            quote!({
                let fields: Vec<String> = vec![#(#types),*];
                format!("{{ {} }}", fields.join("; "))
            })
        };
        metadata.push(quote! {
            ::impasto::abi::EventType {
                name: #key,
                payload: {
                    static PAYLOAD: ::std::sync::OnceLock<String> = ::std::sync::OnceLock::new();
                    PAYLOAD.get_or_init(|| {
                        let mut types = ::impasto::TsTypes::default();
                        let types = &mut types;
                        let _ = &types;
                        #payload
                    })
                },
            }
        });
    }
    Ok(quote! {
        impl ::impasto::ImpastoEvents for #ident {
            fn define_types(types: &mut ::impasto::TsTypes) {
                let _: Vec<String> = vec![#(#declarations),*];
                let _ = types;
            }

            fn declarations() -> &'static [::impasto::abi::EventType] {
                static EVENTS: ::std::sync::OnceLock<Vec<::impasto::abi::EventType>> = ::std::sync::OnceLock::new();
                EVENTS.get_or_init(|| vec![#(#metadata),*])
            }

            fn into_event(self) -> Result<(&'static str, ::impasto::abi::PluginValue), String> {
                match self { #(#encode,)* }
            }
        }
    })
}
