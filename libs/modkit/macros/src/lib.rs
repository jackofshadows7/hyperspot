use heck::ToSnakeCase;
use proc_macro::TokenStream;
use proc_macro2::Span;
use quote::{format_ident, quote};
use syn::{
    parse::Parse, parse::ParseStream, parse_macro_input, punctuated::Punctuated, DeriveInput, Expr,
    Ident, ImplItem, ItemImpl, Lit, LitBool, LitStr, Meta, MetaList, MetaNameValue, Path, Token,
    TypePath,
};

/// Configuration parsed from #[module(...)] attribute
struct ModuleConfig {
    name: String,
    deps: Vec<String>,
    caps: Vec<Capability>,
    ctor: Option<Expr>,             // arbitrary constructor expression
    client: Option<Path>,           // trait path for client DX helpers
    lifecycle: Option<LcModuleCfg>, // optional lifecycle config (on type)
}

#[derive(Debug, PartialEq, Clone)]
enum Capability {
    Db,
    Rest,
    RestHost,
    Stateful,
}

impl Capability {
    const VALID_CAPABILITIES: &'static [&'static str] = &["db", "rest", "rest_host", "stateful"];

    fn suggest_similar(input: &str) -> Vec<&'static str> {
        let mut suggestions: Vec<(&str, f64)> = Self::VALID_CAPABILITIES
            .iter()
            .map(|&cap| (cap, strsim::jaro_winkler(input, cap)))
            .filter(|(_, score)| *score > 0.6) // Only suggest if reasonably similar
            .collect();

        suggestions.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));
        suggestions
            .into_iter()
            .take(2)
            .map(|(cap, _)| cap)
            .collect()
    }

    fn from_ident(ident: &Ident) -> syn::Result<Self> {
        let input = ident.to_string();
        match input.as_str() {
            "db" => Ok(Capability::Db),
            "rest" => Ok(Capability::Rest),
            "rest_host" => Ok(Capability::RestHost),
            "stateful" => Ok(Capability::Stateful),
            other => {
                let suggestions = Self::suggest_similar(other);
                let error_msg = if suggestions.is_empty() {
                    format!("unknown capability '{other}', expected one of: db, rest, rest_host, stateful")
                } else {
                    format!(
                        "unknown capability '{other}'\n       = help: did you mean one of: {}?",
                        suggestions.join(", ")
                    )
                };
                Err(syn::Error::new_spanned(ident, error_msg))
            }
        }
    }

    fn from_str_lit(lit: &LitStr) -> syn::Result<Self> {
        let input = lit.value();
        match input.as_str() {
            "db" => Ok(Capability::Db),
            "rest" => Ok(Capability::Rest),
            "rest_host" => Ok(Capability::RestHost),
            "stateful" => Ok(Capability::Stateful),
            other => {
                let suggestions = Self::suggest_similar(other);
                let error_msg = if suggestions.is_empty() {
                    format!("unknown capability '{other}', expected one of: db, rest, rest_host, stateful")
                } else {
                    format!(
                        "unknown capability '{other}'\n       = help: did you mean one of: {}?",
                        suggestions.join(", ")
                    )
                };
                Err(syn::Error::new_spanned(lit, error_msg))
            }
        }
    }
}

#[derive(Debug, Clone)]
struct LcModuleCfg {
    entry: String,        // entry method name (e.g., "serve")
    stop_timeout: String, // human duration (e.g., "30s")
    await_ready: bool,    // require ReadySignal gating
}

impl Default for LcModuleCfg {
    fn default() -> Self {
        Self {
            entry: "serve".to_string(),
            stop_timeout: "30s".to_string(),
            await_ready: false,
        }
    }
}

impl Parse for ModuleConfig {
    fn parse(input: ParseStream) -> syn::Result<Self> {
        let mut name: Option<String> = None;
        let mut deps: Vec<String> = Vec::new();
        let mut caps: Vec<Capability> = Vec::new();
        let mut ctor: Option<Expr> = None;
        let mut client: Option<Path> = None;
        let mut lifecycle: Option<LcModuleCfg> = None;

        let mut seen_name = false;
        let mut seen_deps = false;
        let mut seen_caps = false;
        let mut seen_ctor = false;
        let mut seen_client = false;
        let mut seen_lifecycle = false;

        let punctuated: Punctuated<Meta, Token![,]> =
            input.parse_terminated(Meta::parse, Token![,])?;

        for meta in punctuated {
            match meta {
                Meta::NameValue(nv) if nv.path.is_ident("name") => {
                    if seen_name {
                        return Err(syn::Error::new_spanned(
                            nv.path,
                            "duplicate `name` parameter",
                        ));
                    }
                    seen_name = true;
                    match nv.value {
                        Expr::Lit(syn::ExprLit {
                            lit: Lit::Str(s), ..
                        }) => {
                            name = Some(s.value());
                        }
                        other => {
                            return Err(syn::Error::new_spanned(
                                other,
                                "name must be a string literal, e.g. name = \"my-module\"",
                            ));
                        }
                    }
                }
                Meta::NameValue(nv) if nv.path.is_ident("ctor") => {
                    if seen_ctor {
                        return Err(syn::Error::new_spanned(
                            nv.path,
                            "duplicate `ctor` parameter",
                        ));
                    }
                    seen_ctor = true;

                    // Reject string literals with a clear message.
                    match &nv.value {
                        Expr::Lit(syn::ExprLit {
                            lit: Lit::Str(s), ..
                        }) => {
                            return Err(syn::Error::new_spanned(
                                s,
                                "ctor must be a Rust expression, not a string literal. \
                 Use: ctor = MyType::new()  (with parentheses), \
                 or:  ctor = Default::default()",
                            ));
                        }
                        _ => {
                            ctor = Some(nv.value.clone());
                        }
                    }
                }
                Meta::NameValue(nv) if nv.path.is_ident("client") => {
                    if seen_client {
                        return Err(syn::Error::new_spanned(
                            nv.path,
                            "duplicate `client` parameter",
                        ));
                    }
                    seen_client = true;
                    let value = nv.value.clone();
                    match value {
                        Expr::Path(ep) => {
                            client = Some(ep.path);
                        }
                        other => {
                            return Err(syn::Error::new_spanned(
                                other,
                                "client must be a trait path, e.g. client = crate::api::MyClient",
                            ));
                        }
                    }
                }
                Meta::NameValue(nv) if nv.path.is_ident("deps") => {
                    if seen_deps {
                        return Err(syn::Error::new_spanned(
                            nv.path,
                            "duplicate `deps` parameter",
                        ));
                    }
                    seen_deps = true;
                    let value = nv.value.clone();
                    match value {
                        Expr::Array(arr) => {
                            for elem in arr.elems {
                                match elem {
                                    Expr::Lit(syn::ExprLit {
                                        lit: Lit::Str(s), ..
                                    }) => {
                                        deps.push(s.value());
                                    }
                                    other => {
                                        return Err(syn::Error::new_spanned(
                                            other,
                                            "deps must be an array of string literals, e.g. deps = [\"db\", \"auth\"]",
                                        ));
                                    }
                                }
                            }
                        }
                        other => {
                            return Err(syn::Error::new_spanned(
                                other,
                                "deps must be an array, e.g. deps = [\"db\", \"auth\"]",
                            ));
                        }
                    }
                }
                Meta::NameValue(nv) if nv.path.is_ident("capabilities") => {
                    if seen_caps {
                        return Err(syn::Error::new_spanned(
                            nv.path,
                            "duplicate `capabilities` parameter",
                        ));
                    }
                    seen_caps = true;
                    let value = nv.value.clone();
                    match value {
                        Expr::Array(arr) => {
                            for elem in arr.elems {
                                match elem {
                                    Expr::Path(ref path) => {
                                        if let Some(ident) = path.path.get_ident() {
                                            caps.push(Capability::from_ident(ident)?);
                                        } else {
                                            return Err(syn::Error::new_spanned(
                                                path,
                                                "capability must be a simple identifier (db, rest, rest_host, stateful)",
                                            ));
                                        }
                                    }
                                    Expr::Lit(syn::ExprLit {
                                        lit: Lit::Str(s), ..
                                    }) => {
                                        caps.push(Capability::from_str_lit(&s)?);
                                    }
                                    other => {
                                        return Err(syn::Error::new_spanned(
                                            other,
                                            "capability must be an identifier or string literal (\"db\", \"rest\", \"rest_host\", \"stateful\")",
                                        ));
                                    }
                                }
                            }
                        }
                        other => {
                            return Err(syn::Error::new_spanned(
                                other,
                                "capabilities must be an array, e.g. capabilities = [db, rest]",
                            ));
                        }
                    }
                }
                // Accept `lifecycle(...)` and also namespaced like `modkit::module::lifecycle(...)`
                Meta::List(list) if path_last_is(&list.path, "lifecycle") => {
                    if seen_lifecycle {
                        return Err(syn::Error::new_spanned(
                            list.path,
                            "duplicate `lifecycle(...)` parameter",
                        ));
                    }
                    seen_lifecycle = true;
                    lifecycle = Some(parse_lifecycle_list(&list)?);
                }
                other => {
                    return Err(syn::Error::new_spanned(
                        other,
                        "unknown attribute parameter",
                    ));
                }
            }
        }

        let name = name.ok_or_else(|| {
            syn::Error::new(
                Span::call_site(),
                "name parameter is required, e.g. #[module(name = \"my-module\", ...)]",
            )
        })?;

        Ok(ModuleConfig {
            name,
            deps,
            caps,
            ctor,
            client,
            lifecycle,
        })
    }
}

fn parse_lifecycle_list(list: &MetaList) -> syn::Result<LcModuleCfg> {
    let mut cfg = LcModuleCfg::default();

    let inner: Punctuated<Meta, Token![,]> =
        list.parse_args_with(Punctuated::<Meta, Token![,]>::parse_terminated)?;

    for m in inner {
        match m {
            Meta::NameValue(MetaNameValue { path, value, .. }) if path.is_ident("entry") => {
                if let Expr::Lit(syn::ExprLit {
                    lit: Lit::Str(s), ..
                }) = value
                {
                    cfg.entry = s.value();
                } else {
                    return Err(syn::Error::new_spanned(
                        value,
                        "entry must be a string literal, e.g. entry = \"serve\"",
                    ));
                }
            }
            Meta::NameValue(MetaNameValue { path, value, .. }) if path.is_ident("stop_timeout") => {
                if let Expr::Lit(syn::ExprLit {
                    lit: Lit::Str(s), ..
                }) = value
                {
                    cfg.stop_timeout = s.value();
                } else {
                    return Err(syn::Error::new_spanned(
                        value,
                        "stop_timeout must be a string literal like \"45s\"",
                    ));
                }
            }
            Meta::Path(p) if p.is_ident("await_ready") => {
                cfg.await_ready = true;
            }
            Meta::NameValue(MetaNameValue { path, value, .. }) if path.is_ident("await_ready") => {
                if let Expr::Lit(syn::ExprLit {
                    lit: Lit::Bool(LitBool { value: b, .. }),
                    ..
                }) = value
                {
                    cfg.await_ready = b;
                } else {
                    return Err(syn::Error::new_spanned(
                        value,
                        "await_ready must be a bool literal (true/false) or a bare flag",
                    ));
                }
            }
            other => {
                return Err(syn::Error::new_spanned(
                    other,
                    "expected lifecycle args: entry=\"...\", stop_timeout=\"...\", await_ready[=true|false]",
                ));
            }
        }
    }

    Ok(cfg)
}

/// Main #[module] attribute macro
///
/// `ctor` must be a Rust expression that evaluates to the module instance,
/// e.g. `ctor = MyModule::new()` or `ctor = Default::default()`.
#[proc_macro_attribute]
pub fn module(attr: TokenStream, item: TokenStream) -> TokenStream {
    let config = parse_macro_input!(attr as ModuleConfig);
    let input = parse_macro_input!(item as DeriveInput);

    // --- Clone all needed pieces early to avoid use-after-move issues ---
    let struct_ident = input.ident.clone();
    let generics_clone = input.generics.clone();
    let (impl_generics, ty_generics, where_clause) = generics_clone.split_for_impl();

    let name_owned: String = config.name.clone();
    let deps_owned: Vec<String> = config.deps.clone();
    let caps_for_asserts: Vec<Capability> = config.caps.clone();
    let caps_for_regs: Vec<Capability> = config.caps.clone();
    let ctor_expr_opt: Option<Expr> = config.ctor.clone();
    let client_trait_opt: Option<Path> = config.client.clone();
    let lifecycle_cfg_opt: Option<LcModuleCfg> = config.lifecycle.clone();

    // Prepare string literals for name/deps
    let name_lit = LitStr::new(&name_owned, Span::call_site());
    let deps_lits: Vec<LitStr> = deps_owned
        .iter()
        .map(|s| LitStr::new(s, Span::call_site()))
        .collect();

    // Constructor expression (provided or Default::default())
    let constructor = if let Some(expr) = &ctor_expr_opt {
        quote! { #expr }
    } else {
        // Use `<T as Default>::default()` so generics/where-clause are honored.
        quote! { <#struct_ident #ty_generics as ::core::default::Default>::default() }
    };

    // Compile-time capability assertions (no calls in consts)
    let mut cap_asserts = Vec::new();

    // Always assert Module is implemented
    cap_asserts.push(quote! {
        const _: () = {
            #[allow(dead_code)]
            fn __modkit_require_Module_impl()
            where
                #struct_ident #ty_generics: ::modkit::contracts::Module,
            {}
        };
    });

    for cap in &caps_for_asserts {
        let q = match cap {
            Capability::Db => quote! {
                const _: () = {
                    #[allow(dead_code)]
                    fn __modkit_require_DbModule_impl()
                    where
                        #struct_ident #ty_generics: ::modkit::contracts::DbModule,
                    {}
                };
            },
            Capability::Rest => quote! {
                const _: () = {
                    #[allow(dead_code)]
                    fn __modkit_require_RestfulModule_impl()
                    where
                        #struct_ident #ty_generics: ::modkit::contracts::RestfulModule,
                    {}
                };
            },
            Capability::RestHost => quote! {
                const _: () = {
                    #[allow(dead_code)]
                    fn __modkit_require_RestHostModule_impl()
                    where
                        #struct_ident #ty_generics: ::modkit::contracts::RestHostModule,
                    {}
                };
            },
            Capability::Stateful => {
                if lifecycle_cfg_opt.is_none() {
                    // Only require direct StatefulModule impl when lifecycle(...) is NOT used.
                    quote! {
                        const _: () = {
                            #[allow(dead_code)]
                            fn __modkit_require_StatefulModule_impl()
                            where
                                #struct_ident #ty_generics: ::modkit::contracts::StatefulModule,
                            {}
                        };
                    }
                } else {
                    quote! {}
                }
            }
        };
        cap_asserts.push(q);
    }

    // Registrator name (avoid lowercasing to reduce collisions)
    let struct_name_snake = struct_ident.to_string().to_snake_case();
    let registrator_name = format_ident!("__{}_registrator", struct_name_snake);

    // === Top-level extras (impl Runnable + optional ready shim) ===
    let mut extra_top_level = proc_macro2::TokenStream::new();

    if let Some(lc) = &lifecycle_cfg_opt {
        // If the type declares lifecycle(...), we generate Runnable at top-level.
        let entry_ident = format_ident!("{}", lc.entry);
        let timeout_ts =
            parse_duration_tokens(&lc.stop_timeout).unwrap_or_else(|e| e.to_compile_error());
        let await_ready_bool = lc.await_ready;

        if await_ready_bool {
            let ready_shim_ident =
                format_ident!("__modkit_run_ready_shim_for_{}", struct_name_snake);

            // Runnable calls entry(cancel, ready). Shim is used by WithLifecycle in ready mode.
            extra_top_level.extend(quote! {
                #[::async_trait::async_trait]
                impl #impl_generics ::modkit::lifecycle::Runnable for #struct_ident #ty_generics #where_clause {
                    async fn run(self: ::std::sync::Arc<Self>, cancel: ::tokio_util::sync::CancellationToken) -> ::anyhow::Result<()> {
                        let (_tx, _rx) = ::tokio::sync::oneshot::channel::<()>();
                        let ready = ::modkit::lifecycle::ReadySignal::from_sender(_tx);
                        self.#entry_ident(cancel, ready).await
                    }
                }

                #[doc(hidden)]
                #[allow(dead_code, non_snake_case)]
                fn #ready_shim_ident(
                    this: ::std::sync::Arc<#struct_ident #ty_generics>,
                    cancel: ::tokio_util::sync::CancellationToken,
                    ready: ::modkit::lifecycle::ReadySignal,
                ) -> ::core::pin::Pin<Box<dyn ::core::future::Future<Output = ::anyhow::Result<()>> + Send>> {
                    Box::pin(async move { this.#entry_ident(cancel, ready).await })
                }
            });

            // Convenience `into_module()` API.
            extra_top_level.extend(quote! {
                impl #impl_generics #struct_ident #ty_generics #where_clause {
                    /// Wrap this instance into a stateful module with lifecycle configuration.
                    pub fn into_module(self) -> ::modkit::lifecycle::WithLifecycle<Self> {
                        ::modkit::lifecycle::WithLifecycle::new(self)
                            .with_stop_timeout(#timeout_ts)
                            .with_ready_mode(true, true, Some(#ready_shim_ident))
                    }
                }
            });
        } else {
            // No ready gating: Runnable calls entry(cancel).
            extra_top_level.extend(quote! {
                #[::async_trait::async_trait]
                impl #impl_generics ::modkit::lifecycle::Runnable for #struct_ident #ty_generics #where_clause {
                    async fn run(self: ::std::sync::Arc<Self>, cancel: ::tokio_util::sync::CancellationToken) -> ::anyhow::Result<()> {
                        self.#entry_ident(cancel).await
                    }
                }

                impl #impl_generics #struct_ident #ty_generics #where_clause {
                    /// Wrap this instance into a stateful module with lifecycle configuration.
                    pub fn into_module(self) -> ::modkit::lifecycle::WithLifecycle<Self> {
                        ::modkit::lifecycle::WithLifecycle::new(self)
                            .with_stop_timeout(#timeout_ts)
                            .with_ready_mode(false, false, None)
                    }
                }
            });
        }
    }

    // Capability registrations (builder API), with special handling for stateful + lifecycle
    let capability_registrations = caps_for_regs.iter().map(|cap| {
        match cap {
            Capability::Db => quote! {
                b.register_db_with_meta(#name_lit,
                    module.clone() as ::std::sync::Arc<dyn ::modkit::contracts::DbModule>);
            },
            Capability::Rest => quote! {
                b.register_rest_with_meta(#name_lit,
                    module.clone() as ::std::sync::Arc<dyn ::modkit::contracts::RestfulModule>);
            },
            Capability::RestHost => quote! {
                b.register_rest_host_with_meta(#name_lit,
                    module.clone() as ::std::sync::Arc<dyn ::modkit::contracts::RestHostModule>);
            },
            Capability::Stateful => {
                if let Some(lc) = &lifecycle_cfg_opt {
                    let timeout_ts = parse_duration_tokens(&lc.stop_timeout)
                        .unwrap_or_else(|e| e.to_compile_error());
                    let await_ready_bool = lc.await_ready;
                    let ready_shim_ident =
                        format_ident!("__modkit_run_ready_shim_for_{}", struct_name_snake);

                    if await_ready_bool {
                        quote! {
                            let wl = ::modkit::lifecycle::WithLifecycle::from_arc(module.clone())
                                .with_stop_timeout(#timeout_ts)
                                .with_ready_mode(true, true, Some(#ready_shim_ident));

                            b.register_stateful_with_meta(
                                #name_lit,
                                ::std::sync::Arc::new(wl) as ::std::sync::Arc<dyn ::modkit::contracts::StatefulModule>
                            );
                        }
                    } else {
                        quote! {
                            let wl = ::modkit::lifecycle::WithLifecycle::from_arc(module.clone())
                                .with_stop_timeout(#timeout_ts)
                                .with_ready_mode(false, false, None);

                            b.register_stateful_with_meta(
                                #name_lit,
                                ::std::sync::Arc::new(wl) as ::std::sync::Arc<dyn ::modkit::contracts::StatefulModule>
                            );
                        }
                    }
                } else {
                    // Alternative path: the type itself must implement StatefulModule
                    quote! {
                        b.register_stateful_with_meta(#name_lit,
                            module.clone() as ::std::sync::Arc<dyn ::modkit::contracts::StatefulModule>);
                    }
                }
            },
        }
    });

    // ClientHub DX helpers (optional)
    let client_code = if let Some(client_trait_path) = &client_trait_opt {
        let snake = name_owned.to_lowercase().replace('-', "_");
        let expose_fn = format_ident!("expose_{}_client", snake);
        let expose_in_fn = format_ident!("expose_{}_client_in", snake);
        let accessor_fn = format_ident!("{}_client", snake);
        let accessor_in_fn = format_ident!("{}_client_in", snake);
        let publish_mock_fn = format_ident!("publish_mock_{}_client", snake);

        quote! {
            // Compile-time trait checks: object-safe + Send + Sync + 'static
            const _: () = {
                fn __modkit_obj_safety<T: ?Sized + ::core::marker::Send + ::core::marker::Sync + 'static>() {}
                let _ = __modkit_obj_safety::<dyn #client_trait_path> as fn();
            };

            impl #impl_generics #struct_ident #ty_generics #where_clause {
                pub const MODULE_NAME: &'static str = #name_lit;
                pub const DEFAULT_SCOPE: &'static str = "global";
            }

            /// Publish this module's typed client under the DEFAULT_SCOPE.
            #[inline]
            pub fn #expose_fn(
                ctx: &::modkit::context::ModuleCtx,
                client: &::std::sync::Arc<dyn #client_trait_path>,
            ) -> ::anyhow::Result<()> {
                ctx.client_hub().register::<dyn #client_trait_path>(client.clone());
                Ok(())
            }

            /// Publish this module's typed client under a custom scope (e.g., tenant).
            #[inline]
            pub fn #expose_in_fn(
                ctx: &::modkit::context::ModuleCtx,
                scope: &str,
                client: &::std::sync::Arc<dyn #client_trait_path>,
            ) -> ::anyhow::Result<()> {
                ctx.client_hub().register_scoped::<dyn #client_trait_path>(scope, client.clone());
                Ok(())
            }

            /// Fetch typed client under DEFAULT_SCOPE (panics with a helpful message if missing).
            #[inline]
            pub fn #accessor_fn(
                hub: &::modkit::client_hub::ClientHub
            ) -> ::std::sync::Arc<dyn #client_trait_path> {
                hub.get::<dyn #client_trait_path>()
                    .expect(concat!(#name_lit, " client not registered; call ",
                                    stringify!(#expose_fn), "(ctx, &client) in provider init()"))
            }

            /// Fetch typed client in custom scope (panics if missing).
            #[inline]
            pub fn #accessor_in_fn(
                hub: &::modkit::client_hub::ClientHub,
                scope: &str
            ) -> ::std::sync::Arc<dyn #client_trait_path> {
                hub.get_scoped::<dyn #client_trait_path>(scope)
                    .expect(concat!(#name_lit, " client (scoped) not registered; call ",
                                    stringify!(#expose_in_fn), "(ctx, scope, &client) in provider init()"))
            }

            /// Dev-only helper to inject mocks quickly.
            #[cfg(test)]
            pub fn #publish_mock_fn(
                hub: &::modkit::client_hub::ClientHub,
                client: ::std::sync::Arc<dyn #client_trait_path>
            ) {
                hub.register::<dyn #client_trait_path>(client);
            }
        }
    } else {
        // Even without a client trait, expose MODULE_NAME for ergonomics.
        quote! {
            impl #impl_generics #struct_ident #ty_generics #where_clause {
                pub const MODULE_NAME: &'static str = #name_lit;
            }
        }
    };

    // Final expansion:
    let expanded = quote! {
        #input

        // Compile-time capability assertions (better errors if trait impls are missing)
        #(#cap_asserts)*

        // Registrator that targets the *builder*, not the final registry
        #[doc(hidden)]
        fn #registrator_name(b: &mut ::modkit::registry::RegistryBuilder) {
            use ::std::sync::Arc;

            let module: Arc<#struct_ident #ty_generics> = Arc::new(#constructor);

            // register core with metadata (name + deps)
            b.register_core_with_meta(
                #name_lit,
                &[#(#deps_lits),*],
                module.clone() as Arc<dyn ::modkit::contracts::Module>
            );

            // capabilities
            #(#capability_registrations)*
        }

        ::inventory::submit! {
            ::modkit::registry::Registrator(#registrator_name)
        }

        #client_code

        // Top-level extras for lifecycle-enabled types (impl Runnable, ready shim, into_module)
        #extra_top_level
    };

    TokenStream::from(expanded)
}

// ============================================================================
// Lifecycle Macro (impl-block attribute) — still supported for opt-in usage
// ============================================================================

#[derive(Debug)]
struct LcCfg {
    method: String,
    stop_timeout: String,
    await_ready: bool,
}

#[proc_macro_attribute]
pub fn lifecycle(attr: TokenStream, item: TokenStream) -> TokenStream {
    let args = parse_macro_input!(attr with Punctuated::<Meta, Token![,]>::parse_terminated);
    let impl_item = parse_macro_input!(item as ItemImpl);

    let cfg = match parse_lifecycle_args(args) {
        Ok(c) => c,
        Err(e) => return e.to_compile_error().into(),
    };

    // Extract impl type ident
    let ty = match &*impl_item.self_ty {
        syn::Type::Path(TypePath { path, .. }) => path.clone(),
        other => {
            return syn::Error::new_spanned(other, "unsupported impl target")
                .to_compile_error()
                .into();
        }
    };

    let runner_ident = format_ident!("{}", cfg.method);
    let mut has_runner = false;
    let mut takes_ready_signal = false;
    for it in &impl_item.items {
        if let ImplItem::Fn(f) = it {
            if f.sig.ident == runner_ident {
                has_runner = true;
                if f.sig.asyncness.is_none() {
                    return syn::Error::new_spanned(f.sig.fn_token, "runner must be async")
                        .to_compile_error()
                        .into();
                }
                let argc = f.sig.inputs.len();
                match argc {
                    2 => {}
                    3 => {
                        if let Some(syn::FnArg::Typed(pat_ty)) = f.sig.inputs.iter().nth(2) {
                            match &*pat_ty.ty {
                                syn::Type::Path(tp) => {
                                    if let Some(seg) = tp.path.segments.last() {
                                        if seg.ident == "ReadySignal" {
                                            takes_ready_signal = true;
                                        } else {
                                            return syn::Error::new_spanned(
                                                &pat_ty.ty,
                                                "third parameter must be ReadySignal when await_ready=true",
                                            )
                                                .to_compile_error()
                                                .into();
                                        }
                                    }
                                }
                                other => {
                                    return syn::Error::new_spanned(
                                        other,
                                        "third parameter must be ReadySignal when await_ready=true",
                                    )
                                    .to_compile_error()
                                    .into();
                                }
                            }
                        }
                    }
                    _ => {
                        return syn::Error::new_spanned(
                            f.sig.inputs.clone(),
                            "invalid runner signature; expected (&self, CancellationToken) or (&self, CancellationToken, ReadySignal)",
                        )
                            .to_compile_error()
                            .into();
                    }
                }
            }
        }
    }
    if !has_runner {
        return syn::Error::new(
            Span::call_site(),
            format!("runner method `{}` not found in impl", cfg.method),
        )
        .to_compile_error()
        .into();
    }

    // Duration literal token
    let timeout_ts = match parse_duration_tokens(&cfg.stop_timeout) {
        Ok(ts) => ts,
        Err(e) => return e.to_compile_error().into(),
    };

    // Generated additions (outside of impl-block)
    let ty_ident = match ty.segments.last() {
        Some(seg) => seg.ident.clone(),
        None => {
            return syn::Error::new_spanned(
                &ty,
                "unsupported impl target: expected a concrete type path",
            )
            .to_compile_error()
            .into();
        }
    };
    let ty_snake = ty_ident.to_string().to_snake_case();

    let ready_shim_ident = format_ident!("__modkit_run_ready_shim{ty_snake}");
    let await_ready_bool = cfg.await_ready;

    let extra = if takes_ready_signal {
        quote! {
            #[async_trait::async_trait]
            impl ::modkit::lifecycle::Runnable for #ty {
                async fn run(self: ::std::sync::Arc<Self>, cancel: ::tokio_util::sync::CancellationToken) -> ::anyhow::Result<()> {
                    let (_tx, _rx) = ::tokio::sync::oneshot::channel::<()>();
                    let ready = ::modkit::lifecycle::ReadySignal::from_sender(_tx);
                    self.#runner_ident(cancel, ready).await
                }
            }

            #[doc(hidden)]
            #[allow(non_snake_case, dead_code)]
            fn #ready_shim_ident(
                this: ::std::sync::Arc<#ty>,
                cancel: ::tokio_util::sync::CancellationToken,
                ready: ::modkit::lifecycle::ReadySignal,
            ) -> ::core::pin::Pin<Box<dyn ::core::future::Future<Output = ::anyhow::Result<()>> + Send>> {
                Box::pin(async move { this.#runner_ident(cancel, ready).await })
            }

            impl #ty {
                /// Converts this value into a stateful module wrapper with configured stop-timeout.
                pub fn into_module(self) -> ::modkit::lifecycle::WithLifecycle<Self> {
                    ::modkit::lifecycle::WithLifecycle::new(self)
                        .with_stop_timeout(#timeout_ts)
                        .with_ready_mode(#await_ready_bool, true, Some(#ready_shim_ident))
                }
            }
        }
    } else {
        quote! {
            #[async_trait::async_trait]
            impl ::modkit::lifecycle::Runnable for #ty {
                async fn run(self: ::std::sync::Arc<Self>, cancel: ::tokio_util::sync::CancellationToken) -> ::anyhow::Result<()> {
                    self.#runner_ident(cancel).await
                }
            }

            impl #ty {
                /// Converts this value into a stateful module wrapper with configured stop-timeout.
                pub fn into_module(self) -> ::modkit::lifecycle::WithLifecycle<Self> {
                    ::modkit::lifecycle::WithLifecycle::new(self)
                        .with_stop_timeout(#timeout_ts)
                        .with_ready_mode(#await_ready_bool, false, None)
                }
            }
        }
    };

    let out = quote! {
        #impl_item
        #extra
    };
    out.into()
}

fn parse_lifecycle_args(args: Punctuated<Meta, Token![,]>) -> syn::Result<LcCfg> {
    let mut method: Option<String> = None;
    let mut stop_timeout = "30s".to_string();
    let mut await_ready = false;

    for m in args {
        match m {
            Meta::NameValue(nv) if nv.path.is_ident("method") => {
                if let Expr::Lit(el) = nv.value {
                    if let Lit::Str(s) = el.lit {
                        method = Some(s.value());
                    } else {
                        return Err(syn::Error::new_spanned(
                            el,
                            "method must be a string literal",
                        ));
                    }
                } else {
                    return Err(syn::Error::new_spanned(
                        nv,
                        "method must be a string literal",
                    ));
                }
            }
            Meta::NameValue(nv) if nv.path.is_ident("stop_timeout") => {
                if let Expr::Lit(el) = nv.value {
                    if let Lit::Str(s) = el.lit {
                        stop_timeout = s.value();
                    } else {
                        return Err(syn::Error::new_spanned(
                            el,
                            "stop_timeout must be a string literal like \"45s\"",
                        ));
                    }
                } else {
                    return Err(syn::Error::new_spanned(
                        nv,
                        "stop_timeout must be a string literal like \"45s\"",
                    ));
                }
            }
            Meta::NameValue(nv) if nv.path.is_ident("await_ready") => {
                if let Expr::Lit(el) = nv.value {
                    if let Lit::Bool(b) = el.lit {
                        await_ready = b.value();
                    } else {
                        return Err(syn::Error::new_spanned(
                            el,
                            "await_ready must be a bool literal (true/false)",
                        ));
                    }
                } else {
                    return Err(syn::Error::new_spanned(
                        nv,
                        "await_ready must be a bool literal (true/false)",
                    ));
                }
            }
            Meta::Path(p) if p.is_ident("await_ready") => {
                await_ready = true;
            }
            other => {
                return Err(syn::Error::new_spanned(other, "expected named args: method=\"...\", stop_timeout=\"...\", await_ready=true|false"));
            }
        }
    }

    let method = method.ok_or_else(|| {
        syn::Error::new(
            Span::call_site(),
            "missing required arg: method=\"runner_name\"",
        )
    })?;
    Ok(LcCfg {
        method,
        stop_timeout,
        await_ready,
    })
}

fn parse_duration_tokens(s: &str) -> syn::Result<proc_macro2::TokenStream> {
    let err = || {
        syn::Error::new(
            Span::call_site(),
            format!("invalid duration: {s}. Use e.g. \"500ms\", \"45s\", \"2m\", \"1h\""),
        )
    };
    if let Some(stripped) = s.strip_suffix("ms") {
        let v: u64 = stripped.parse().map_err(|_| err())?;
        Ok(quote! { ::std::time::Duration::from_millis(#v) })
    } else if let Some(stripped) = s.strip_suffix('s') {
        let v: u64 = stripped.parse().map_err(|_| err())?;
        Ok(quote! { ::std::time::Duration::from_secs(#v) })
    } else if let Some(stripped) = s.strip_suffix('m') {
        let v: u64 = stripped.parse().map_err(|_| err())?;
        Ok(quote! { ::std::time::Duration::from_secs(#v * 60) })
    } else if let Some(stripped) = s.strip_suffix('h') {
        let v: u64 = stripped.parse().map_err(|_| err())?;
        Ok(quote! { ::std::time::Duration::from_secs(#v * 3600) })
    } else {
        Err(err())
    }
}

fn path_last_is(path: &syn::Path, want: &str) -> bool {
    path.segments
        .last()
        .map(|s| s.ident == want)
        .unwrap_or(false)
}
