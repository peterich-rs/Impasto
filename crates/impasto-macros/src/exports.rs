use std::collections::BTreeSet;

use heck::ToLowerCamelCase;
use proc_macro2::TokenStream;
use quote::{format_ident, quote};
use syn::{
    Error, FnArg, ImplItem, ImplItemFn, ItemImpl, ItemStruct, LitStr, Pat, Path, Result,
    ReturnType, Type,
};

use crate::{type_arguments, type_named};

#[derive(Default)]
struct ModuleOptions {
    name: Option<LitStr>,
    specifier: Option<LitStr>,
    init: Option<syn::Ident>,
    events: Option<Path>,
}

pub fn module(args: TokenStream, item: ItemStruct) -> Result<TokenStream> {
    let mut options = ModuleOptions::default();
    syn::parse::Parser::parse2(
        syn::meta::parser(|meta| {
            if meta.path.is_ident("specifier") {
                options.specifier = Some(meta.value()?.parse()?);
            } else if meta.path.is_ident("name") {
                options.name = Some(meta.value()?.parse()?);
            } else if meta.path.is_ident("init") {
                options.init = Some(meta.value()?.parse()?);
            } else if meta.path.is_ident("events") {
                options.events = Some(meta.value()?.parse()?);
            } else {
                return Err(meta.error("unknown module option"));
            }
            Ok(())
        }),
        args,
    )?;
    let ident = &item.ident;
    let specifier = options
        .specifier
        .ok_or_else(|| Error::new_spanned(ident, "module needs a specifier"))?;
    let name = options
        .name
        .unwrap_or_else(|| LitStr::new(&ident.to_string(), ident.span()));
    let init = match options.init {
        Some(init) => quote!(#ident::#init(ctx)?),
        None if matches!(item.fields, syn::Fields::Unit) => quote!(#ident),
        None => {
            return Err(Error::new_spanned(
                ident,
                "a stateful module needs init = function",
            ));
        }
    };
    let events = options.events.as_ref().map_or_else(
        || quote!(&[]),
        |events| quote!(<#events as ::impasto::ImpastoEvents>::declarations()),
    );
    let event_context = options.events.as_ref().map(|events| {
        quote! {
            let scoped_context = ctx.with_event_schema::<#events>();
            let ctx = &scoped_context;
        }
    });
    let event_types = options
        .events
        .as_ref()
        .map(|events| quote!(<#events as ::impasto::ImpastoEvents>::define_types(&mut types);));
    let plugin = format_ident!("__{}ImpastoPlugin", ident);
    Ok(quote! {
        #item

        #[doc(hidden)]
        struct #plugin;

        impl #ident {
            pub fn plugin() -> impl ::impasto::ImpastoPlugin {
                #plugin
            }

            fn __impasto_module(owner: Option<::std::sync::Arc<Self>>) -> Result<::impasto::abi::Module, ::impasto::PluginInitError> {
                let mut types = ::impasto::TsTypes::default();
                let methods = Self::__impasto_bindings(owner, None, &mut types);
                #event_types
                let declarations = types.render()?;
                static DECLARATIONS: ::std::sync::OnceLock<String> = ::std::sync::OnceLock::new();
                Ok(::impasto::abi::Module {
                    name: #name,
                    specifier: #specifier,
                    methods,
                    exports: ::impasto::abi::ModuleExports {
                        typescript: DECLARATIONS.get_or_init(|| declarations),
                        events: #events,
                        object: false,
                    },
                })
            }
        }

        impl ::impasto::ImpastoPlugin for #plugin {
            fn describe(&self) -> Result<::impasto::ModuleDescriptor, ::impasto::PluginInitError> {
                ::impasto::ModuleDescriptor::from_module(&#ident::__impasto_module(None)?)
            }

            fn build_module(&self, ctx: &::impasto::PluginContext) -> Result<::impasto::abi::Module, ::impasto::PluginInitError> {
                #event_context
                let _ = ctx;
                let instance = #init;
                #ident::__impasto_module(Some(::std::sync::Arc::new(instance)))
            }
        }
    })
}

pub fn object(item: ItemStruct, local: bool) -> Result<TokenStream> {
    let ident = &item.ident;
    let name = ident.to_string();
    let (resource, bindings) = if local {
        (
            quote!(Box::new(self)),
            quote!(Self::__impasto_local_metadata(types)),
        )
    } else {
        (
            quote!({
                let owner = ::std::sync::Arc::new(self);
                Box::new(::impasto::BoundResource::new(
                    Self::__IMPASTO_METHODS,
                    move |cancel| {
                        Self::__impasto_bindings(
                            Some(owner),
                            Some(cancel),
                            &mut ::impasto::TsTypes::default(),
                        )
                    },
                ))
            }),
            quote!(Self::__impasto_bindings(None, None, types)),
        )
    };
    Ok(quote! {
        #item

        impl ::impasto::NativeObject for #ident {
            fn into_resource(self) -> Box<dyn ::impasto::abi::NativeResource> {
                #resource
            }

            fn typescript(types: &mut ::impasto::TsTypes) -> String {
                types.define::<Self>(#name, |types| {
                    use ::impasto::abi::Invoke;
                    let methods = #bindings;
                    let mut declarations = vec!["  close(): void;".to_owned()];
                    for method in methods {
                        if let Some(types) = method.types {
                            let returns = if matches!(method.invoke, Invoke::Promise(_)) {
                                format!("Promise<{}>", types.returns)
                            } else {
                                types.returns.to_owned()
                            };
                            declarations.push(format!("  {}({}): {};", method.name, types.parameters, returns));
                        }
                    }
                    format!("export interface {} {{\n{}\n}}", #name, declarations.join("\n"))
                });
                #name.into()
            }
        }
    })
}

pub fn export(mut item: ItemImpl, local: bool) -> Result<TokenStream> {
    if item.trait_.is_some() || !item.generics.params.is_empty() {
        return Err(Error::new_spanned(
            item,
            "export requires a concrete inherent impl",
        ));
    }
    let mut entries = Vec::new();
    let mut names = BTreeSet::new();
    for member in &mut item.items {
        let ImplItem::Fn(method) = member else {
            continue;
        };
        if !matches!(method.vis, syn::Visibility::Public(_)) {
            continue;
        }
        let entry = Binding::parse(method, local)?;
        if !names.insert(entry.name.clone()) {
            return Err(Error::new_spanned(
                method,
                format!("duplicate export {:?}", entry.name),
            ));
        }
        entries.push(entry);
    }
    if local {
        return Ok(local_object(item, &entries));
    }
    let names = entries.iter().map(|entry| &entry.name);
    let methods = entries.iter().map(Binding::build);
    let owner = &item.self_ty;
    Ok(quote! {
        #item

        impl #owner {
            const __IMPASTO_METHODS: &'static [&'static str] = &[#(#names),*];

            fn __impasto_bindings(
                owner: Option<::std::sync::Arc<Self>>,
                lifetime: Option<::impasto::CancelToken>,
                types: &mut ::impasto::TsTypes,
            ) -> Vec<::impasto::abi::Method> {
                let _ = (&owner, &lifetime, &types);
                vec![#(#methods),*]
            }
        }
    })
}

fn local_object(item: ItemImpl, entries: &[Binding]) -> TokenStream {
    let owner = &item.self_ty;
    let names = entries.iter().map(|entry| &entry.name);
    let declarations = entries.iter().map(|entry| entry.build_with(true));
    let dispatch = entries.iter().map(Binding::local_call);
    quote! {
        #item

        impl #owner {
            fn __impasto_local_metadata(types: &mut ::impasto::TsTypes) -> Vec<::impasto::abi::Method> {
                let _ = types;
                vec![#(#declarations),*]
            }
        }

        impl ::impasto::abi::NativeResource for #owner {
            fn methods(&self) -> &'static [&'static str] {
                &[#(#names),*]
            }

            fn call(
                &mut self, method: &str, args: &[::impasto::abi::PluginValue], _: &::impasto::CancelToken,
            ) -> Result<::impasto::abi::PluginValue, String> {
                match method {
                    #(#dispatch,)*
                    other => Err(format!("unknown resource method {other:?}")),
                }
            }
        }
    }
}

#[derive(Default)]
struct Options {
    rename: Option<String>,
    blocking: bool,
    main_thread: bool,
    awaits_user: bool,
    resource: bool,
}

impl Options {
    fn parse(method: &mut ImplItemFn) -> Result<Self> {
        let mut options = Self::default();
        let mut retained = Vec::with_capacity(method.attrs.len());
        for attr in std::mem::take(&mut method.attrs) {
            if !attr
                .path()
                .segments
                .last()
                .is_some_and(|part| part.ident == "method")
            {
                retained.push(attr);
                continue;
            }
            attr.parse_nested_meta(|meta| {
                if meta.path.is_ident("rename") {
                    options.rename = Some(meta.value()?.parse::<LitStr>()?.value());
                } else if meta.path.is_ident("blocking") {
                    options.blocking = true;
                } else if meta.path.is_ident("main_thread") {
                    options.main_thread = true;
                } else if meta.path.is_ident("awaits_user") {
                    options.awaits_user = true;
                } else if meta.path.is_ident("resource") {
                    options.resource = true;
                } else {
                    return Err(meta.error("unknown method option"));
                }
                Ok(())
            })?;
        }
        method.attrs = retained;
        Ok(options)
    }
}

struct Arg {
    ident: syn::Ident,
    ty: Type,
    borrowed: bool,
}

struct Binding {
    ident: syn::Ident,
    name: String,
    args: Vec<Arg>,
    receiver: bool,
    mode: Mode,
    result: Type,
    fallible: bool,
    unit: bool,
    resource: bool,
    awaits_user: bool,
}

enum Mode {
    Inline,
    Async,
    Blocking,
    Main,
    Stream(Box<Type>),
}

impl Binding {
    fn parse(method: &mut ImplItemFn, local: bool) -> Result<Self> {
        let options = Options::parse(method)?;
        let sig = &method.sig;
        if !sig.generics.params.is_empty()
            || sig.unsafety.is_some()
            || sig.abi.is_some()
            || sig.variadic.is_some()
        {
            return Err(Error::new_spanned(sig, "unsupported export signature"));
        }
        let receiver = sig.receiver().is_some();
        if let Some(receiver) = sig.receiver()
            && (receiver.reference.is_none() || (!local && receiver.mutability.is_some()))
        {
            return Err(Error::new_spanned(
                receiver,
                "exports use &self or an associated function",
            ));
        }
        let mut args = Vec::new();
        let mut stream = None;
        for param in &sig.inputs {
            let FnArg::Typed(param) = param else { continue };
            let Pat::Ident(ident) = &*param.pat else {
                return Err(Error::new_spanned(
                    param,
                    "export parameters must have names",
                ));
            };
            let ty = &*param.ty;
            let borrowed = if let Type::Reference(reference) = ty {
                if !type_named(&reference.elem, "str") || reference.mutability.is_some() {
                    return Err(Error::new_spanned(ty, "only &str can be borrowed"));
                }
                true
            } else {
                false
            };
            args.push(Arg {
                ident: ident.ident.clone(),
                ty: ty.clone(),
                borrowed,
            });
        }
        if let [.., sink, cancel] = args.as_slice()
            && let Some(item) =
                type_arguments(&sink.ty, "TypedSink").and_then(|items| items.first().copied())
            && type_named(&cancel.ty, "CancelToken")
        {
            stream = Some(item.clone());
            args.truncate(args.len() - 2);
        }
        let asynchronous = sig.asyncness.is_some();
        if (options.blocking || options.main_thread) && asynchronous
            || options.blocking && options.main_thread
            || stream.is_some() && (!asynchronous || options.resource || options.awaits_user)
        {
            return Err(Error::new_spanned(sig, "incompatible native call options"));
        }
        let mode = if let Some(item) = stream {
            Mode::Stream(Box::new(item))
        } else if asynchronous {
            Mode::Async
        } else if options.blocking {
            Mode::Blocking
        } else if options.main_thread {
            Mode::Main
        } else {
            Mode::Inline
        };
        if local && (!matches!(mode, Mode::Inline) || options.resource) {
            return Err(Error::new_spanned(
                sig,
                "local object methods return synchronous values",
            ));
        }
        if matches!(mode, Mode::Inline) && options.awaits_user {
            return Err(Error::new_spanned(
                sig,
                "awaits_user requires a Promise method",
            ));
        }
        if !matches!(mode, Mode::Inline) && args.iter().any(|arg| arg.borrowed) {
            return Err(Error::new_spanned(
                sig,
                "scheduled methods must accept owned values",
            ));
        }
        let ty = match &sig.output {
            ReturnType::Default => syn::parse_quote!(()),
            ReturnType::Type(_, ty) => *ty.clone(),
        };
        let inner = type_arguments(&ty, "Result").and_then(|items| items.first().copied());
        let result = inner.unwrap_or(&ty).clone();
        let unit = matches!(&result, Type::Tuple(tuple) if tuple.elems.is_empty());
        if matches!(result, Type::Reference(_)) || (matches!(mode, Mode::Stream(_)) && !unit) {
            return Err(Error::new_spanned(
                result,
                "invalid native method return type",
            ));
        }
        let name = options
            .rename
            .unwrap_or_else(|| sig.ident.to_string().to_lower_camel_case());
        if matches!(
            name.as_str(),
            "close" | "call" | "__proto__" | "constructor"
        ) {
            return Err(Error::new_spanned(
                sig,
                "export name is reserved by native resources",
            ));
        }
        Ok(Self {
            ident: sig.ident.clone(),
            name,
            args,
            receiver,
            mode,
            result,
            fallible: inner.is_some(),
            unit,
            resource: options.resource,
            awaits_user: options.awaits_user,
        })
    }

    fn local_call(&self) -> TokenStream {
        let ident = &self.ident;
        let name = &self.name;
        let decoded = self.args.iter().enumerate().map(|(index, arg)| {
            let ident = format_ident!("__impasto_param_{index}");
            let name = arg.ident.to_string().to_lower_camel_case();
            let ty = &arg.ty;
            let ty = if arg.borrowed {
                quote!(String)
            } else {
                quote!(#ty)
            };
            quote!(let #ident = ::impasto::macro_support::decode_arg::<#ty>(args, #index, #name)?;)
        });
        let function = if self.receiver {
            quote!(self.#ident)
        } else {
            quote!(Self::#ident)
        };
        let args = self.args.iter().enumerate().map(|(index, arg)| {
            let ident = format_ident!("__impasto_param_{index}");
            if arg.borrowed {
                quote!(&#ident)
            } else {
                quote!(#ident)
            }
        });
        let check = self
            .fallible
            .then(|| quote!(.map_err(::impasto::macro_support::encode_error)?));
        quote!(#name => {
            #(#decoded)*
            ::impasto::IntoPluginValue::into_value(#function(#(#args),*) #check)
        })
    }

    fn build(&self) -> TokenStream {
        self.build_with(false)
    }

    fn build_with(&self, metadata_only: bool) -> TokenStream {
        let Self {
            ident,
            name,
            result,
            ..
        } = self;
        let receiver =
            (self.receiver && !metadata_only).then(|| quote!(let owner = owner.clone();));
        let owner = self.receiver.then(|| {
            quote! {
                let owner = owner.as_ref().cloned().ok_or("module owner is unavailable")?;
            }
        });
        let function = if self.receiver {
            quote!(owner.#ident)
        } else {
            quote!(Self::#ident)
        };
        let decoded = self.args.iter().enumerate().map(|(index, arg)| {
            let ident = format_ident!("__impasto_param_{index}");
            let name = arg.ident.to_string().to_lower_camel_case();
            let ty = &arg.ty;
            let ty = if arg.borrowed {
                quote!(String)
            } else {
                quote!(#ty)
            };
            quote!(let #ident = ::impasto::macro_support::decode_arg::<#ty>(&args, #index, #name)?;)
        });
        let decoded = quote!(#owner #(#decoded)*);
        let vars = (0..self.args.len())
            .map(|index| format_ident!("__impasto_param_{index}"))
            .collect::<Vec<_>>();
        let arguments = self.args.iter().zip(&vars).map(|(arg, ident)| {
            if arg.borrowed {
                quote!(&#ident)
            } else {
                quote!(#ident)
            }
        });
        let mut arguments = arguments.collect::<Vec<_>>();
        if matches!(self.mode, Mode::Stream(_)) {
            arguments.extend([
                quote!(::impasto::TypedSink::new(sink)),
                quote!(cancel.clone()),
            ]);
        }
        let await_it = matches!(self.mode, Mode::Async | Mode::Stream(_)).then(|| quote!(.await));
        let check = self
            .fallible
            .then(|| quote!(.map_err(::impasto::macro_support::encode_error)?));
        let call = quote!(#function(#(#arguments),*) #await_it #check);
        let ret = if self.resource && matches!(self.mode, Mode::Inline) {
            quote!(Ok(::impasto::NativeObject::into_resource(#call)))
        } else if self.resource {
            quote!(Ok(::impasto::abi::PluginResult::Resource(
                ::impasto::abi::ResourceFactory::new(move || {
                    ::impasto::NativeObject::into_resource(value)
                })
            )))
        } else if matches!(self.mode, Mode::Stream(_)) {
            if self.fallible {
                quote!(#function(#(#arguments),*).await.map_err(::impasto::macro_support::encode_error))
            } else {
                quote!({ #call; Ok(()) })
            }
        } else {
            quote!(::impasto::IntoPluginValue::into_value(#call))
        };
        let finish = if self.resource {
            quote!(
                outcome
                    .unwrap_or_else(|message| ::impasto::abi::PluginResult::Rejected { message })
            )
        } else {
            quote!(::impasto::macro_support::finish_value(outcome))
        };
        let mut bindings = vars.iter().map(|var| quote!(#var)).collect::<Vec<_>>();
        if self.receiver {
            bindings.insert(0, quote!(owner));
        }

        let constructor = match self.mode {
            Mode::Inline => {
                let constructor = if self.resource {
                    quote!(resource)
                } else {
                    quote!(sync)
                };
                quote!(Method::#constructor(#name, move |args| {
                    if lifetime.as_ref().is_some_and(::impasto::CancelToken::is_cancelled) {
                        return Err("native resource is closed".into());
                    }
                    #decoded
                    #ret
                }))
            }
            Mode::Async => {
                let ret = if self.resource {
                    quote!(let value = #call; #ret)
                } else {
                    ret
                };
                quote! {
                    Method::promise(#name, move |args, settle, exec| {
                        let prepared = (|| { #decoded Ok::<_, String>((#(#bindings,)*)) })();
                        let (#(#bindings,)*) = match prepared {
                            Ok(args) => args,
                            Err(message) => return settle.send(::impasto::abi::PluginResult::Rejected { message }),
                        };
                        let request = settle.cancellation();
                        let scope = lifetime.clone();
                        let settle = settle.with_scope(scope.clone());
                        exec.spawn(Box::pin(async move {
                            let task = async move { #ret };
                            if let Some(outcome) = ::impasto::macro_support::scoped(task, request, scope).await {
                                settle.send(#finish);
                            }
                        }));
                    })
                }
            }
            Mode::Blocking | Mode::Main => {
                let dispatch = if matches!(self.mode, Mode::Main) {
                    quote!(main_thread)
                } else {
                    quote!(worker)
                };
                let ret = if self.resource {
                    quote!(let value = #call; #ret)
                } else {
                    ret
                };
                quote! {
                    Method::promise(#name, move |args, settle, exec| {
                        let prepared = (|| { #decoded Ok::<_, String>((#(#bindings,)*)) })();
                        let (#(#bindings,)*) = match prepared {
                            Ok(args) => args,
                            Err(message) => return settle.send(::impasto::abi::PluginResult::Rejected { message }),
                        };
                        let request = settle.cancellation();
                        let scope = lifetime.clone();
                        let settle = settle.with_scope(scope.clone());
                        exec.#dispatch(settle, Box::new(move || {
                            if request.is_cancelled() || scope.as_ref().is_some_and(::impasto::CancelToken::is_cancelled) {
                                return ::impasto::abi::PluginResult::Rejected { message: "request cancelled".into() };
                            }
                            let outcome = (|| { #ret })();
                            #finish
                        }));
                    })
                }
            }
            Mode::Stream(_) => quote! {
                Method::stream_with_exec(#name, move |args, mut sink, exec| {
                    let prepared = (|| { #decoded Ok::<_, String>((#(#bindings,)*)) })();
                    let (#(#bindings,)*) = match prepared {
                        Ok(args) => args,
                        Err(message) => return sink.fail(message),
                    };
                    let cancel = sink.cancellation();
                    let terminal = sink.terminal_guard();
                    let scope = lifetime.clone();
                    exec.spawn(Box::pin(async move {
                        let task = async { #ret };
                        match ::impasto::macro_support::scoped(task, cancel.clone(), scope).await {
                            Some(Ok(())) => terminal.finish(),
                            Some(Err(message)) => terminal.fail(message),
                            None => terminal.finish(),
                        }
                    }));
                })
            },
        };

        let (constructor, lifetime) = if metadata_only {
            (
                quote!(Method::sync(#name, |_| Err("declaration-only native method".into()))),
                None,
            )
        } else {
            (
                constructor,
                Some(quote! {
                    let lifetime = lifetime.clone();
                    let _ = &lifetime;
                }),
            )
        };
        let mut declarations = self.args.iter().enumerate().map(|(index, arg)| {
            let name = arg.ident.to_string().to_lower_camel_case();
            let ty = &arg.ty;
            let ty = if arg.borrowed { quote!(String) } else { quote!(#ty) };
            let optional = self.args[index..].iter().all(|arg| type_arguments(&arg.ty, "Option").is_some());
            let optional = if optional { "?" } else { "" };
            quote!(format!("{}{}: {}", #name, #optional, <#ty as ::impasto::ImpastoType>::input_typescript(types)))
        }).collect::<Vec<_>>();
        let returns = if let Mode::Stream(item) = &self.mode {
            declarations.push(quote!("signal?: AbortSignal".to_owned()));
            quote!(
                format!("ReadableStream<{}>", <#item as ::impasto::ImpastoType>::typescript(types))
            )
        } else if self.resource {
            quote!(<#result as ::impasto::NativeObject>::typescript(types))
        } else if self.unit {
            quote!("void".to_owned())
        } else {
            quote!(<#result as ::impasto::ImpastoType>::typescript(types))
        };
        let awaits_user = self.awaits_user.then(|| quote!(.awaits_user()));
        quote!({
            use ::impasto::abi::Method;
            #receiver
            #lifetime
            let parameters: Vec<String> = vec![#(#declarations),*];
            let returns = #returns;
            static TYPES: ::std::sync::OnceLock<(String, String)> = ::std::sync::OnceLock::new();
            let (parameters, returns) = TYPES.get_or_init(|| (parameters.join(", "), returns));
            #constructor.with_types(parameters, returns).strict_arguments() #awaits_user
        })
    }
}
