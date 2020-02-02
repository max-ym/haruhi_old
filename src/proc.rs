use std::fmt::Debug;
use crate::ResponseCode;

#[derive(Debug)]
pub struct RequestContext {

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

    type R: ResponseContext;

    async fn handle(self, req: RequestContext) -> Self::R;
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
