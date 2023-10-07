extern crate proc_macro;

use proc_macro::TokenStream as TS;

use darling::ast::NestedMeta;
use darling::FromMeta;
use proc_macro2::{Span, TokenStream};
use quote::{format_ident, quote};
use syn::parse::{Parse, ParseBuffer};
use syn::punctuated::Punctuated;
use syn::Token;
use syn::{parse_quote, Expr, ExprLit, ItemFn, Lit, LitInt, ReturnType, Type};

struct Args {
    meta: Vec<NestedMeta>,
}

impl Parse for Args {
    fn parse(input: &ParseBuffer) -> syn::Result<Self> {
        let meta = Punctuated::<NestedMeta, Token![,]>::parse_terminated(input)?;
        Ok(Args {
            meta: meta.into_iter().collect(),
        })
    }
}

#[derive(Debug, FromMeta)]
struct Args2 {
    #[darling(default)]
    pool_size: Option<syn::Expr>,
    #[darling(default)]
    pool_size_env: Option<syn::LitStr>,
}

#[derive(Debug, FromMeta)]
struct ReplicateArgs {
    #[darling(default)]
    count: Option<syn::LitInt>
}


#[derive(Debug, FromMeta)]
struct EntryArgs {
    #[darling(default)]
    replicate: Option<syn::LitInt>
}

fn task_pool_run(args: &[NestedMeta], f: syn::ItemFn) -> Result<TokenStream, TokenStream> {
    let args = Args2::from_list(args).map_err(|e| e.write_errors())?;

    let pool_size_env = match &args.pool_size_env {
        Some(lit) => {
            let env = lit.value().to_string();
            if let Ok(var) = std::env::var(env) {
                Some(var)
            } else {
                None
            }
        }
        None => None,
    };

    let pool_size = match &args.pool_size {
        Some(Expr::Lit(ExprLit { ref lit, attrs: _ })) => match lit {
            Lit::Str(v) => match v.parse::<LitInt>() {
                Ok(parsed_pool_size) => Expr::Lit(ExprLit {
                    attrs: vec![],
                    lit: Lit::Int(parsed_pool_size),
                }),
                Err(_) => {
                    let err = syn::Error::new(Span::call_site(), "is not valid number");
                    return Err(syn::Error::to_compile_error(&err));
                }
            },
            Lit::Int(v) => Expr::Lit(ExprLit {
                attrs: vec![],
                lit: Lit::Int(v.clone()),
            }),
            _ => {
                let err = syn::Error::new(
                    Span::call_site(),
                    "only integer and string literals are allowed",
                );
                return Err(syn::Error::to_compile_error(&err));
            }
        },
        Some(Expr::Macro(macr)) => Expr::Macro(macr.clone()),
        Some(_) => {
            let err = syn::Error::new(Span::call_site(), "only literals and macros are allowed");
            return Err(syn::Error::to_compile_error(&err));
        }
        None => {
            let litval = match &pool_size_env {
                Some(val) => val.clone(),
                None => "1".to_string(),
            };

            Expr::Lit(ExprLit {
                attrs: vec![],
                lit: Lit::Int(LitInt::new(&litval, Span::call_site())),
            })
        }
    };

    if f.sig.asyncness.is_none() {
        let err = syn::Error::new_spanned(&f.sig, "task functions must be async");
        return Err(syn::Error::to_compile_error(&err));
    }
    if !f.sig.generics.params.is_empty() {
        let err = syn::Error::new_spanned(&f.sig, "task functions must not be generic");
        return Err(syn::Error::to_compile_error(&err));
    }
    if f.sig.generics.where_clause.is_some() {
        let err = syn::Error::new_spanned(&f.sig, "task functions must not have `where` clauses");
        return Err(syn::Error::to_compile_error(&err));
    }
    if f.sig.abi.is_some() {
        let err = syn::Error::new_spanned(&f.sig, "task functions must not have an ABI qualifier");
        return Err(syn::Error::to_compile_error(&err));
    }
    if f.sig.variadic.is_some() {
        let err = syn::Error::new_spanned(&f.sig, "task functions must not be variadic");
        return Err(syn::Error::to_compile_error(&err));
    }
    match &f.sig.output {
        ReturnType::Default => {}
        ReturnType::Type(_, ty) => match &**ty {
            Type::Tuple(tuple) if tuple.elems.is_empty() => {}
            Type::Never(_) => {}
            _ => {
                let err = syn::Error::new_spanned(
                    &f.sig,
                    "task functions must either not return a value, return `()` or return `!`",
                );
                return Err(syn::Error::to_compile_error(&err));
            }
        },
    }

    let mut arg_names = Vec::new();
    let mut fargs = f.sig.inputs.clone();

    for arg in fargs.iter_mut() {
        match arg {
            syn::FnArg::Receiver(_) => {
                let err =
                    syn::Error::new_spanned(arg, "task functions must not have receiver arguments");
                return Err(syn::Error::to_compile_error(&err));
            }
            syn::FnArg::Typed(t) => match t.pat.as_mut() {
                syn::Pat::Ident(id) => {
                    arg_names.push(id.ident.clone());
                    id.mutability = None;
                }
                _ => {
                    let err = syn::Error::new_spanned(
                        arg,
                        "pattern matching in task arguments is not yet supported",
                    );
                    return Err(syn::Error::to_compile_error(&err));
                }
            },
        }
    }

    let task_ident = f.sig.ident.clone();
    let task_inner_ident = format_ident!("__{}_task", task_ident);

    let mut task_inner = f;
    let visibility = task_inner.vis.clone();
    task_inner.vis = syn::Visibility::Inherited;
    task_inner.sig.ident = task_inner_ident.clone();

    let mut task_outer: ItemFn = parse_quote! {
        #visibility fn #task_ident(#fargs) -> Option<::reika::executor::core::TaskRef> {
            type Fut = impl ::core::future::Future + 'static;
            const POOL_SIZE: usize = #pool_size;
            static mut POOL: ::reika::executor::core::TaskPool<Fut, POOL_SIZE> = ::reika::executor::core::TaskPool::new();
            unsafe { POOL.prepare_task(move || #task_inner_ident(#(#arg_names,)*)) }
        }
    };

    task_outer.attrs.append(&mut task_inner.attrs.clone());

    let result = quote! {
        // This is the user's task function, renamed.
        // We put it outside the #task_ident fn below, because otherwise
        // the items defined there (such as STORAGE...) would be in scope
        // in the user's code.
        #[doc(hidden)]
        #task_inner

        #task_outer
    };

    Ok(result)
}

fn replicate_run(args: &[NestedMeta], f: syn::ItemFn) -> Result<TokenStream, TokenStream> {
    let args = ReplicateArgs::from_list(args).map_err(|e| e.write_errors())?;
    let count = args.count.unwrap_or(LitInt::new("1", Span::call_site()));
    let count = count.base10_parse::<usize>().unwrap();

    if f.sig.asyncness.is_some() {
        let err = syn::Error::new_spanned(&f.sig, "replicated non main function must not be async");
        return Err(syn::Error::to_compile_error(&err));
    }
    if !f.sig.generics.params.is_empty() {
        let err = syn::Error::new_spanned(&f.sig, "replicated function must not be generic");
        return Err(syn::Error::to_compile_error(&err));
    }
    if f.sig.generics.where_clause.is_some() {
        let err = syn::Error::new_spanned(&f.sig, "replicated function must not have `where` clauses");
        return Err(syn::Error::to_compile_error(&err));
    }
    if f.sig.abi.is_some() {
        let err = syn::Error::new_spanned(&f.sig, "replicated function must not have an ABI qualifier");
        return Err(syn::Error::to_compile_error(&err));
    }
    if f.sig.variadic.is_some() {
        let err = syn::Error::new_spanned(&f.sig, "replicated function must not be variadic");
        return Err(syn::Error::to_compile_error(&err));
    }
    match &f.sig.output {
        ReturnType::Default => {}
        ReturnType::Type(_, ty) => match &**ty {
            Type::Tuple(tuple) if tuple.elems.is_empty() => {}
            Type::Never(_) => {}
            _ => {
                let err = syn::Error::new_spanned(
                    &f.sig,
                    "replicated functions must either not return a value, return `()` or return `!`",
                );
                return Err(syn::Error::to_compile_error(&err));
            }
        },
    }

    let mut arg_names = Vec::new();
    let mut fargs = f.sig.inputs.clone();

    for arg in fargs.iter_mut() {
        match arg {
            syn::FnArg::Receiver(_) => {
                let err =
                    syn::Error::new_spanned(arg, "replicated functions must not have receiver arguments");
                return Err(syn::Error::to_compile_error(&err));
            }
            syn::FnArg::Typed(t) => match t.pat.as_mut() {
                syn::Pat::Ident(id) => {
                    arg_names.push(id.ident.clone());
                    id.mutability = None;
                }
                _ => {
                    let err = syn::Error::new_spanned(
                        arg,
                        "pattern matching in replicated function arguments is not yet supported",
                    );
                    return Err(syn::Error::to_compile_error(&err));
                }
            },
        }
    }

    // Extract the function name and other details
    let fn_name = &f.sig.ident;

    // Generate new function names and functions
    let new_fns = (1..=count).map(|i| {
        let mut newfn = f.clone();
        let core = i - 1;
        let pinstmt: syn::Stmt = syn::parse2(quote!{
            ::reika::util::set_cpu_affinity(#core);
        }).expect("failed to parse affinity statement");

        newfn.block.stmts.insert(0, pinstmt);

        if i == 1 {
            newfn.sig.ident = f.sig.ident.clone();

            let mut thread_spawns = Vec::new();

            for j in 2..=count {
                let subsequent_fn = format_ident!("{}_{}", fn_name, j);

                let spawn_stmt: syn::Stmt = syn::parse2(quote! {
                    ::std::thread::spawn(#subsequent_fn);
                }).expect("Failed to parse thread spawn statement");
                
                thread_spawns.push(spawn_stmt);
            }

            // Prepend the new thread spawn statements to the original body
            let original_stmts = &mut newfn.block.stmts;
            for stmt in thread_spawns.into_iter().rev() {
                original_stmts.insert(0, stmt);
            }
        } else {
            newfn.sig.ident = format_ident!("{}_{}", fn_name, i);
        }

        quote! {
            #newfn
        }
    });

    // Convert quote output into TokenStream
    let result = quote! {
        #(#new_fns)*
    };

    Ok(result)
}

fn entry_run(args: &[NestedMeta], mut f: syn::ItemFn) -> Result<TokenStream, TokenStream> {
    let args = EntryArgs::from_list(args).map_err(|e| e.write_errors())?;
    let replicate = args.replicate.unwrap_or(LitInt::new("1", Span::call_site()));
    let replicate = replicate.base10_parse::<usize>().unwrap();

    if f.sig.asyncness.is_none() {
        let err = syn::Error::new_spanned(&f.sig, "entry must be marked async");
        return Err(syn::Error::to_compile_error(&err));
    }
    if !f.sig.generics.params.is_empty() {
        let err = syn::Error::new_spanned(&f.sig, "entry must not be generic");
        return Err(syn::Error::to_compile_error(&err));
    }
    if f.sig.generics.where_clause.is_some() {
        let err = syn::Error::new_spanned(&f.sig, "entry must not have `where` clauses");
        return Err(syn::Error::to_compile_error(&err));
    }
    if f.sig.abi.is_some() {
        let err = syn::Error::new_spanned(&f.sig, "entry must not have an ABI qualifier");
        return Err(syn::Error::to_compile_error(&err));
    }
    if f.sig.variadic.is_some() {
        let err = syn::Error::new_spanned(&f.sig, "entry must not be variadic");
        return Err(syn::Error::to_compile_error(&err));
    }
    match &f.sig.output {
        ReturnType::Default => {}
        ReturnType::Type(_, ty) => match &**ty {
            Type::Tuple(tuple) if tuple.elems.is_empty() => {}
            Type::Never(_) => {}
            _ => {
                let err = syn::Error::new_spanned(
                    &f.sig,
                    "entry must either not return a value, return `()` or return `!`",
                );
                return Err(syn::Error::to_compile_error(&err));
            }
        },
    }
    if !&f.sig.inputs.is_empty() {
        let err = syn::Error::new_spanned(&f.sig, "entry must not have any arguments");
        return Err(syn::Error::to_compile_error(&err));
    }

    // Extract the function name and other details
    let entry_fn_name = &f.sig.ident;

    // Generate new function names and functions
    let new_fns = (1..=replicate).map(|i| {
        let mut newfn = f.clone();

        let core = i - 1;
        let pinstmt: syn::Stmt = syn::parse2(quote!{
            ::reika::util::set_cpu_affinity(#core);
        }).expect("failed to parse affinity statement");

        // pin the thread to a core
        newfn.block.stmts.insert(0, pinstmt);

        if i == 1 {
            // First one gets to keep the name of the actual function
            newfn.sig.ident = f.sig.ident.clone();

            let mut thread_spawns = Vec::new();

            for j in 2..=replicate {
                let subsequent_fn = format_ident!("{}_{}", entry_fn_name, j);

                let spawn_stmt: syn::Stmt = syn::parse2(quote! {
                    ::std::thread::spawn(#subsequent_fn);
                }).expect("Failed to parse thread spawn statement");
                
                thread_spawns.push(spawn_stmt);
            }

            // Prepend the new thread spawn statements to the original body
            let original_stmts = &mut newfn.block.stmts;
            for stmt in thread_spawns.into_iter().rev() {
                original_stmts.insert(0, stmt);
            }
        } else {
            newfn.sig.ident = format_ident!("{}_{}", entry_fn_name, i);
        }

        // Generate wrapper functions
        let inner_fn_ident = format_ident!("{}_inner", newfn.sig.ident);
        let outer_fn_ident = newfn.sig.ident.clone();
        newfn.sig.ident = inner_fn_ident.clone();

        let outer_fn_definition: ItemFn = parse_quote! {
            fn #outer_fn_ident() {
                type Fut = impl ::core::future::Future + 'static;
                const POOL_SIZE: usize = 1;
                static mut POOL: ::reika::executor::core::TaskPool<Fut, POOL_SIZE> = ::reika::executor::core::TaskPool::new();
                let task = unsafe { POOL.prepare_task(move || #inner_fn_ident()).unwrap() };

                ::reika::executor::PerThreadExecutor::spawn_task(task);
                ::reika::executor::PerThreadExecutor::run(Some(|| {
                    if ::reika::reactor::PerThreadReactor::run(1000).is_err() {
                        println!("failed to start reika reactor")
                    }
                }));
            }
        };

        quote! {
            #[doc(hidden)]
            #newfn

            #outer_fn_definition
        }
    });

    // Convert quote output into TokenStream
    let result = quote! {
        #(#new_fns)*
    };

    Ok(result)
}

#[proc_macro_attribute]
pub fn task(args: TS, item: TS) -> TS {
    let args = syn::parse_macro_input!(args as Args);
    let f = syn::parse_macro_input!(item as syn::ItemFn);

    task_pool_run(&args.meta, f).unwrap_or_else(|x| x).into()
}

#[proc_macro_derive(Future)]
pub fn derive_future(input: TS) -> TS {
    let syn::DeriveInput {
        ident, generics, ..
    } = syn::parse_macro_input!(input);

    let inner = quote! {
        type Output = ::std::io::Result<i32>;

        fn poll(
            mut self: ::std::pin::Pin<&mut Self>,
            ctx: &mut ::std::task::Context<'_>,
        ) -> ::std::task::Poll<Self::Output> {
            if let Some(return_val) = self.req.return_val {
                if return_val < 0 {
                    return ::std::task::Poll::Ready(Err(::std::io::Error::from_raw_os_error(-return_val)));
                }

                return ::std::task::Poll::Ready(Ok(return_val));
            }

            self.req.waker = Some(ctx.waker().clone());

            unsafe {
                if self.reactor.submit(&mut self.req).is_err() {
                    // enqueue immediately
                    ctx.waker().wake_by_ref();
                }
            }

            ::std::task::Poll::Pending
        }
    };

    let output = quote! {
        impl #generics ::std::future::Future for #ident #generics {
            #inner
        }
    };

    output.into()
}

#[proc_macro_attribute]
pub fn replicate(args: TS, item: TS) -> TS {
    let args = syn::parse_macro_input!(args as Args);
    let f = syn::parse_macro_input!(item as syn::ItemFn);

    replicate_run(&args.meta, f).unwrap_or_else(|x| x).into()
}

#[proc_macro_attribute]
pub fn entry(args: TS, item: TS) -> TS {
    let args = syn::parse_macro_input!(args as Args);
    let f = syn::parse_macro_input!(item as syn::ItemFn);

    entry_run(&args.meta, f).unwrap_or_else(|x| x).into()
}