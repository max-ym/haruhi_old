use std::fmt::{Debug, Formatter, Error};
use crate::ResponseCode;
use std::pin::Pin;
use std::cell::Cell;
use std::sync::Mutex;

#[derive(Debug)]
pub struct RequestContext {

    url_info: UrlInfo,
}

/// Pointer to string slice inside of a `UrlInfo`.
#[derive(Clone, Copy)]
pub struct StrRef(*const u8, usize);

impl From<(*const u8, usize)> for StrRef {

    fn from(tuple: (*const u8, usize)) -> Self {
        StrRef(tuple.0, tuple.1)
    }
}

impl From<&[u8]> for StrRef {

    fn from(slice: &[u8]) -> Self {
        (slice.as_ptr(), slice.len()).into()
    }
}

impl AsRef<str> for StrRef {

    fn as_ref(&self) -> &str {
        unsafe {
            let slice = std::slice::from_raw_parts(self.0, self.1);
            std::str::from_utf8_unchecked(slice)
        }
    }
}

impl Debug for StrRef {

    fn fmt(&self, f: &mut Formatter) -> Result<(), Error> {
        self.as_ref().fmt(f)
    }
}

unsafe impl Send for StrRef {}
unsafe impl Sync for StrRef {}

struct UrlInfo {

    /// Original URL as it was delivered to the server.
    /// # Example
    /// For `http://example.com/admin/new?q=something` result is `/admin/new?q=something`.
    original_url: Pin<String>,

    parse_mutex: Mutex<()>,

    /// Parsed parameters in the URL. Is lazily loaded.
    params: Cell<Vec<Param>>,

    /// Full path in the URL excluding parameters. Is lazily loaded.
    /// # Example
    /// For `http://example.com/admin/new?q=something` result is `/admin/new`.
    path: Cell<Option<StrRef>>,

    parts: Cell<Vec<StrRef>>,
}

impl Debug for UrlInfo {

    fn fmt(&self, f: &mut Formatter<'_>) -> Result<(), Error> {
        write!(
            f,
            "UrlInfo {{ \
            original_url: {:?}, \
            params: {:?}, \
            path: {:?}, \
            parts: {:?}}}",
            self.original_url,
            unsafe { &*self.params.as_ptr() },
            self.path.get().unwrap(),
            unsafe { &*self.parts.as_ptr() },
        )
    }
}

#[derive(Debug, Copy, Clone)]
pub struct Param {

    name: Option<StrRef>,

    value: Option<StrRef>,
}

impl UrlInfo {

    pub fn is_lazy_parsed(&self) -> bool {
        self.path.get().is_some()
    }

    pub fn parse_if_needed(&self) {
        if !self.is_lazy_parsed() {
            self.lazy_parse();
        }
    }

    /// Force lazy parsing of the URL to get the parameters and url path parts.
    pub fn lazy_parse(&self) {
        if self.original_url.is_empty() {
            // Nothing to parse.
            return;
        }

        let _lock = self.parse_mutex.lock().unwrap();

        let path_parts = self.original_url.split('/');
        let parts: Vec<StrRef> = {
            let mut vec = Vec::with_capacity(path_parts.size_hint().0);
            for part in path_parts {
                vec.push(part.as_bytes().into());
            }
            vec
        };
        let last = parts.last().unwrap().clone();
        let mut params_parts = last.as_ref().split('?');
        let path = params_parts.next().unwrap();
        let params = {
            let mut vec = Vec::with_capacity(params_parts.size_hint().0);
            for part in params_parts {
                let starts_with_eq = part.starts_with('=');
                let mut split = part.split('=');
                let ptr = |v: &str| v.as_bytes().into();
                let p = if starts_with_eq {
                    Param {
                        name: None,
                        value: split.next().map(ptr),
                    }
                } else {
                    Param {
                        name: split.next().map(ptr),
                        value: split.next().map(ptr),
                    }
                };
                vec.push(p);
            }
            vec
        };

        self.params.set(params);
        self.path.set(Some(path.as_bytes().into()));
        self.parts.set(parts);
    }
}

unsafe impl Sync for UrlInfo {}

impl RequestContext {

    pub fn original_url(&self) -> &str {
        &self.url_info.original_url
    }

    pub fn params(&self) -> &Vec<Param> {
        self.url_info.parse_if_needed();
        unsafe { &*self.url_info.params.as_ptr() }
    }

    pub fn parts(&self) -> &Vec<StrRef> {
        self.url_info.parse_if_needed();
        unsafe { &*self.url_info.parts.as_ptr() }
    }

    pub fn path(&self) -> StrRef {
        self.url_info.parse_if_needed();
        self.url_info.path.get().unwrap()
    }
}

#[derive(Debug)]
pub struct ContextBundle<RS>
    where RS: ResultContext {
    result: RS,
    request: RequestContext,
}

#[derive(Debug)]
pub enum ResultContextBundle<O, E>
    where O: OkResultContext, E: ErrResultContext {
    Ok {
        result: O,
        request: RequestContext,
    },
    Err {
        result: E,
        request: RequestContext,
    }
}

impl<O, E> ResultContextBundle<O, E>
    where O: OkResultContext, E: ErrResultContext {

    fn fail_err_unwrap(&self) -> ! {
        panic!("`Ok` unwrap attempted on `Err` variant of {:?}", &self)
    }

    fn fail_fix_good(&self) -> ! {
        panic!("Attempted fix on a `Ok` context: {:?}", &self)
    }

    fn fail_unwrap_err_on_good(&self) -> ! {
        panic!("Attempted `Err` unwrap on a `Ok` context: {:?}", &self)
    }

    pub fn unwrap(self) -> ContextBundle<O> {
        use ResultContextBundle::*;
        if let Ok { result, request } = self {
            ContextBundle { result, request }
        } else {
            self.fail_err_unwrap()
        }
    }

    /// Try to fix the context by applying given update.
    ///
    /// # Panics
    /// Panic arises when function is called over a healthy context.
    pub async fn amend<U: Update>(self, update: U) -> ResultContextBundle<O, E> {
        if let ResultContextBundle::Err { result, request } = self {
            match result.apply(update) {
                Err(e) => ResultContextBundle::Err {
                    result: e,
                    request,
                },
                Ok(v) => ResultContextBundle::Ok {
                    result: v,
                    request,
                }
            }
        } else {
            self.fail_fix_good()
        }
    }

    pub fn is_ok(&self) -> bool {
        use ResultContextBundle::*;
        match self {
            Ok { result: _, request: _ } => true,
            Err { result: _, request: _ } => false,
        }
    }

    pub fn is_err(&self) -> bool {
        !self.is_ok()
    }

    pub fn err(&self) -> &E {
        if let ResultContextBundle::Err { result, request: _ } = &self {
            result
        } else {
            self.fail_err_unwrap()
        }
    }

    pub fn unwrap_err(self) -> ContextBundle<E> {
        use ResultContextBundle::*;
        if let Err { result, request } = self {
            ContextBundle {
                result,
                request
            }
        } else {
            self.fail_unwrap_err_on_good()
        }
    }

    /// Update the value using provided Process.
    pub async fn update<P, U>(self, process: P) -> ResultContextBundle<O, E> where
        U: Update,
        P: Process<RS=O, Result=U> + Send + Sync {
        process.exec_over(self.unwrap()).await
    }

    /// Update the value using provided process. If the value is broken then fix it before
    /// update using process.
    pub async fn update_fixed<P, F, U>(self, process: P, fix: F) -> ResultContextBundle<O, E> where
        U: Update,
        P: Process<RS=O, Result=U> + Send + Sync,
        F: Process<RS=E, Result=U> + Send + Sync {
        self.continue_or_fix(fix).await
            .update(process).await
    }

    /// Continue the flow if context is valid or attempt the fix if it is broken.
    pub async fn continue_or_fix<F, U>(self, fix: F) -> ResultContextBundle<O, E> where
        U: Update,
        F: Process<RS=E, Result=U> + Send + Sync {
        if self.is_err() {
            self.fix(fix).await
        } else {
            self
        }
    }

    /// Attempt to fix broken context.
    ///
    /// # Panics
    /// Panic will arise if context is already healthy.
    pub async fn fix<F, U>(self, fix: F) -> ResultContextBundle<O, E> where
        U: Update,
        F: Process<RS=E, Result=U> + Send + Sync {
        let unwrap = self.unwrap_err();
        let update = fix.exec(&unwrap).await;
        let update_context = ResultContextBundle::Err {
            result: unwrap.result,
            request: unwrap.request,
        };
        update_context.amend::<U>(update).await.into()
    }
}

/// Update to the `ResultContext` that should be applied after executing `Process`.
pub trait Update {}

#[async_trait]
pub trait Process: Sized {

    type RS: ResultContext;
    type Result: Update;

    /// Execute this process and get `Update` object.
    async fn exec(self, context: &ContextBundle<Self::RS>) -> Self::Result;

    /// Execute this process and apply produced `Update` to current context.
    async fn exec_over<O, E>(self, context: ContextBundle<Self::RS>)
                             -> ResultContextBundle<O, E> where
        O: OkResultContext,
        E: ErrResultContext {
        let result = self.exec(&context).await;
        let result = context.result.apply(result);
        match result {
            Ok(v) => ResultContextBundle::Ok {
                result: v,
                request: context.request,
            },
            Err(e) => ResultContextBundle::Err {
                result: e,
                request: context.request,
            },
        }
    }
}

/// Full description of route handling.
#[async_trait]
pub trait RouteHandle {

    async fn handle(&self, req: RequestContext) -> Box<dyn Data>;
}

#[async_trait]
pub trait OkResultContext: ResultContext {}

#[async_trait]
pub trait ErrResultContext: ResultContext {}

/// Holds data generated by multiple `Process` and is used at the end of the process to generate a
/// response.
pub trait ResultContext: Send + Sync + Debug {

    /// Apply given update. Current result is consumed to be transformed into
    /// type that combines previous result state and update.
    /// Function can either emit Ok or Err. Err means that resulting context
    /// is holding error which should be handled before other updates could be applied.
    fn apply<U, R, E>(self, update: U) -> Result<R, E>
        where U: Update, R: OkResultContext, E: ErrResultContext;
}

/// Context that can be translated into response.
pub trait ResponseContext: ResultContext {

    type Data: Data;

    fn code(&self) -> ResponseCode;

    fn to_data(&self) -> Self::Data;
}

pub trait Data: Debug {

    fn code(&self) -> ResponseCode;

    fn into_bytes(self) -> Vec<u8>;
}
