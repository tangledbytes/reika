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
}

fn task_pool_run(args: &[NestedMeta], f: syn::ItemFn) -> Result<TokenStream, TokenStream> {
    let args = Args2::from_list(args).map_err(|e| e.write_errors())?;

    let pool_size = args.pool_size.unwrap_or(Expr::Lit(ExprLit {
        attrs: vec![],
        lit: Lit::Int(LitInt::new("1", Span::call_site())),
    }));

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
        #visibility fn #task_ident(#fargs) -> Option<::async_executor::TaskRef> {
            type Fut = impl ::core::future::Future + 'static;
            const POOL_SIZE: usize = #pool_size;
            static mut POOL: ::async_executor::TaskPool<Fut, POOL_SIZE> = ::async_executor::TaskPool::new();
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
        type Output = std::io::Result<i32>;

        fn poll(
            mut self: std::pin::Pin<&mut Self>,
            ctx: &mut std::task::Context<'_>,
        ) -> std::task::Poll<Self::Output> {
            if let Some(return_val) = self.req.return_val {
                if return_val < 0 {
                    return std::task::Poll::Ready(Err(std::io::Error::from_raw_os_error(-return_val)));
                }

                return std::task::Poll::Ready(Ok(return_val));
            }

            self.req.waker = Some(ctx.waker().clone());

            unsafe {
                if self.reactor.submit(&mut self.req).is_err() {
                    // enqueue immediately
                    ctx.waker().wake_by_ref();
                }
            }

            std::task::Poll::Pending
        }
    };

    let output = quote! {
        impl #generics std::future::Future for #ident #generics {
            #inner
        }
    };

    output.into()
}
