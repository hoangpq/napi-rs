extern crate proc_macro;

use proc_macro::TokenStream;
use proc_macro2::{Ident, Literal};
use quote::{format_ident, quote};
use syn::fold::{fold_fn_arg, fold_signature, Fold};
use syn::parse::{Parse, ParseStream, Result};
use syn::punctuated::Punctuated;
use syn::{parse_macro_input, Block, FnArg, ItemFn, Signature, Token};

struct ArgLength {
  length: Option<Literal>,
}

impl Parse for ArgLength {
  fn parse(input: ParseStream) -> Result<Self> {
    let vars = Punctuated::<Literal, Token![,]>::parse_terminated(input)?;
    Ok(ArgLength {
      length: vars.first().map(|i| i.clone()),
    })
  }
}

struct JsFunction {
  args: Vec<FnArg>,
  name: Option<Ident>,
  signature: Option<Signature>,
  block: Vec<Block>,
}

impl JsFunction {
  pub fn new() -> Self {
    JsFunction {
      args: vec![],
      name: None,
      signature: None,
      block: vec![],
    }
  }
}

impl Fold for JsFunction {
  fn fold_fn_arg(&mut self, arg: FnArg) -> FnArg {
    self.args.push(arg.clone());
    fold_fn_arg(self, arg)
  }

  fn fold_signature(&mut self, signature: Signature) -> Signature {
    self.name = Some(format_ident!("{}", signature.ident));
    let mut new_signature = signature.clone();
    new_signature.ident = format_ident!("_{}", signature.ident);
    self.signature = Some(new_signature);
    fold_signature(self, signature)
  }

  fn fold_block(&mut self, node: Block) -> Block {
    self.block.push(node.clone());
    node
  }
}

#[proc_macro_attribute]
pub fn js_function(attr: TokenStream, input: TokenStream) -> TokenStream {
  let arg_len = parse_macro_input!(attr as ArgLength);
  let arg_len_span = arg_len.length.unwrap_or(Literal::usize_unsuffixed(0));
  let input = parse_macro_input!(input as ItemFn);
  let mut js_fn = JsFunction::new();
  js_fn.fold_item_fn(input);
  let fn_name = js_fn.name.unwrap();
  let fn_block = js_fn.block;
  let signature = js_fn.signature.unwrap();
  let new_fn_name = signature.ident.clone();
  let expanded = quote! {
    #signature #(#fn_block)*

    extern "C" fn #fn_name(
      raw_env: napi_rs::sys::napi_env,
      cb_info: napi_rs::sys::napi_callback_info,
    ) -> napi_rs::sys::napi_value {
      use std::io::Write;
      use std::mem;
      use std::os::raw::c_char;
      use std::ptr;
      use napi_rs::{Any, Env, Status, Value, CallContext};
      let mut argc = #arg_len_span as usize;
      let mut raw_args =
      unsafe { mem::MaybeUninit::<[napi_rs::sys::napi_value; 8]>::uninit().assume_init() };
      let mut raw_this = ptr::null_mut();

      let mut has_error = false;

      unsafe {
        let status = napi_rs::sys::napi_get_cb_info(
          raw_env,
          cb_info,
          &mut argc as *mut usize as *mut u64,
          &mut raw_args[0],
          &mut raw_this,
          ptr::null_mut(),
        );
        has_error = has_error && (Status::from(status) == Status::Ok);
      }

      let env = Env::from_raw(raw_env);
      let call_ctx = CallContext::new(&env, raw_this, raw_args, #arg_len_span);
      let result = call_ctx.and_then(|ctx| #new_fn_name(ctx));
      has_error = has_error && result.is_err();

      match result {
        Ok(result) => result.into_raw(),
        Err(e) => {
          if !cfg!(windows) {
            let _ = writeln!(::std::io::stderr(), "Error calling function: {:?}", e);
          }
          let message = format!("{:?}", e);
          unsafe {
            napi_rs::sys::napi_throw_error(raw_env, ptr::null(), message.as_ptr() as *const c_char);
          }
          let mut undefined = ptr::null_mut();
          unsafe { napi_rs::sys::napi_get_undefined(raw_env, &mut undefined) };
          undefined
        }
      }
    }
  };
  // Hand the output tokens back to the compiler
  TokenStream::from(expanded)
}
