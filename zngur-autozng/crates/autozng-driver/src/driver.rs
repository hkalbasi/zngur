use rustc_driver::{Callbacks, Compilation, run_compiler};
use rustc_interface::interface;
use rustc_middle::ty::TyCtxt;
use rustc_public::CompilerError;
use rustc_public::rustc_internal;
use std::cell::Cell;
use std::ops::ControlFlow;

use scoped_tls::scoped_thread_local;

use crate::config::Config;

pub struct DriverContext<'tcx, 'cfg> {
    pub tcx: TyCtxt<'tcx>,
    pub cfg: &'cfg Config,
}

scoped_thread_local!(
    static TCX: Cell<*const ()>
);

#[derive(Debug, Default)]
pub struct AnalyzeReport {
    pub errors: usize,
}

pub struct RustcPublic<F>
where
    F: for<'tcx> FnMut(TyCtxt<'tcx>) -> ControlFlow<AnalyzeReport> + Send,
{
    result: Option<ControlFlow<AnalyzeReport>>,
    analyze_callback: F,
    cfg: Config,
}

impl<F> RustcPublic<F>
where
    F: for<'tcx> FnMut(TyCtxt<'tcx>) -> ControlFlow<AnalyzeReport> + Send,
{
    /// Creates a new `RustcPublic` instance, with given test_function and arguments.
    pub fn new(callback: F) -> Self {
        RustcPublic {
            result: None,
            analyze_callback: callback,
            cfg: Config::default(),
        }
    }

    /// Runs the compiler against given target and tests it with `test_function`
    pub fn run(&mut self, args: &[String]) -> Result<(), CompilerError<AnalyzeReport>> {
        let compiler_result = rustc_driver::catch_fatal_errors(|| -> interface::Result<()> {
            run_compiler(args, self);
            Ok(())
        });
        match (compiler_result, self.result.take()) {
            (Ok(Ok(())), Some(ControlFlow::Continue(()))) => {
                // emit ZNG here
                eprintln!("Emitting ZNG");
                Ok(())
            }
            (Ok(Ok(())), Some(ControlFlow::Break(report))) => {
                eprintln!("Errors during analysis?");
                Err(CompilerError::Interrupted(report))
            }
            (Ok(Ok(())), None) => {
                eprintln!("Errors before analysis step");
                Err(CompilerError::Skipped)
            }
            (Ok(Err(_)), _) | (Err(_), _) => {
                eprintln!("Errors in compiler");
                Err(CompilerError::Failed)
            }
        }
    }
}

impl<F> Callbacks for RustcPublic<F>
where
    F: for<'tcx> FnMut(TyCtxt<'tcx>) -> ControlFlow<AnalyzeReport> + Send,
{
    /// Called after analysis. Return value instructs the compiler whether to
    /// continue the compilation afterwards (defaults to `Compilation::Continue`)
    fn after_analysis<'tcx>(
        &mut self,
        _compiler: &interface::Compiler,
        tcx: TyCtxt<'tcx>,
    ) -> Compilation {
        rustc_internal::run(tcx, || {
            let driver_ctx = &DriverContext {
                tcx,
                cfg: &self.cfg,
            };
            set_ctxt(driver_ctx, || {
                self.result = Some((self.analyze_callback)(tcx));
            })
        })
        .unwrap();

        // stop after analysis
        Compilation::Stop
    }
}

/// This is here so that the tcx and some configuration values can be accessed from trait extensions on rustc_public
/// to implement missing functionality
/// so long as it is only called within the control flow of the driver's analysis it's completely valid
pub(crate) fn with_ctxt<R>(f: impl for<'tcx, 'cfg> FnOnce(&DriverContext<'tcx, 'cfg>) -> R) -> R {
    assert!(TCX.is_set());
    TCX.with(|tcx_cell| {
        let ptr = tcx_cell.get();
        assert!(!ptr.is_null());
        // SAFETY the cell is set and not null, and we control where it is set above
        let tcx = unsafe { *(ptr as *const &DriverContext<'_, '_>) };
        f(tcx)
    })
}

pub(crate) fn set_ctxt<R>(ctx: &DriverContext, f: impl for<'tcx> FnOnce() -> R) -> R {
    let ctx_ptr: *const () = (&raw const ctx) as _;
    TCX.set(&Cell::new(ctx_ptr), f)
}
