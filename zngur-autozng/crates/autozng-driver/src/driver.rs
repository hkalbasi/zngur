//! Thin wrapper around `rustc_driver::run_compiler` that runs autozng's analysis after the
//! compiler's analysis phase and stops compilation before codegen.
//!
//! [`RustcPublic::run`] drives the compile; the closure passed to `run_compiler` is this
//! struct itself, whose [`Callbacks::after_analysis`] invokes the user-supplied analyze
//! callback and stashes the owned render result. `run` then writes the resulting `.zng`
//! (see `render.rs`).

use rustc_driver::{Callbacks, Compilation, run_compiler};
use rustc_interface::interface;
use rustc_middle::ty::TyCtxt;
use rustc_public::CompilerError;
use rustc_public::rustc_internal;
use std::cell::{Cell, RefCell};
use std::ops::ControlFlow;
use std::path::PathBuf;

use scoped_tls::scoped_thread_local;
use zngur_def::ZngurSpec;

use crate::config::Config;
use crate::render;
use crate::rustc_ext::VisiblePaths;

/// Ambient state made available to [`with_ctxt`] for the duration of the analysis callback.
pub struct DriverContext<'tcx, 'cfg> {
    pub tcx: TyCtxt<'tcx>,
    pub cfg: &'cfg Config,
    /// The re-export map, built on first use (see [`DriverContext::visible_path`]).
    visible_paths: RefCell<Option<VisiblePaths>>,
}

impl DriverContext<'_, '_> {
    /// The path a definition is *visible* by when a public `pub use` re-export exposes it under a
    /// different path than its definition path, else `None`.
    ///
    /// Built once per analysis and cached.
    pub(crate) fn visible_path(&self, def_id: rustc_span::def_id::DefId) -> Option<Vec<String>> {
        if self.visible_paths.borrow().is_none() {
            *self.visible_paths.borrow_mut() = Some(crate::rustc_ext::compute(self.tcx));
        }
        self.visible_paths
            .borrow()
            .as_ref()
            .and_then(|map| map.get(&def_id).cloned())
    }
}

// Type-erased `&DriverContext<'_, '_>` (as a raw pointer) so `with_ctxt` can recover it
// without naming the `'tcx`/`'cfg` lifetimes. Only set inside `after_analysis`.
scoped_thread_local!(
    static TCX: Cell<*const ()>
);

/// Result of a completed analysis pass, carried out of `after_analysis` via `ControlFlow`.
///
/// Currently only counts hard errors seen by the `DiagCtxt`; warnings emitted during
/// analysis are not reflected here.
#[derive(Debug, Default)]
pub struct AnalyzeReport {
    #[allow(dead_code)] // counted for the report; not read by the caller yet
    pub errors: usize,
}

pub struct RustcPublic<F>
where
    F: for<'tcx> FnMut(TyCtxt<'tcx>) -> ControlFlow<AnalyzeReport, ZngurSpec> + Send,
{
    result: Option<ControlFlow<AnalyzeReport, ZngurSpec>>,
    analyze_callback: F,
    cfg: Config,
    /// Where to write the `.zng`, decided in `after_analysis` (the crate name is only
    /// available while the `TyCtxt` is alive).
    output_path: Option<PathBuf>,
}

impl<F> RustcPublic<F>
where
    F: for<'tcx> FnMut(TyCtxt<'tcx>) -> ControlFlow<AnalyzeReport, ZngurSpec> + Send,
{
    /// Creates a new `RustcPublic` instance, with given test_function and arguments.
    pub fn new(callback: F, cfg: Config) -> Self {
        RustcPublic {
            result: None,
            analyze_callback: callback,
            cfg,
            output_path: None,
        }
    }

    /// Runs the compiler against given target and tests it with `test_function`
    pub fn run(&mut self, args: &[String]) -> Result<(), CompilerError<AnalyzeReport>> {
        let compiler_result = rustc_driver::catch_fatal_errors(|| -> interface::Result<()> {
            run_compiler(args, self);
            Ok(())
        });
        match (compiler_result, self.result.take()) {
            (Ok(Ok(())), Some(ControlFlow::Continue(spec))) => {
                // The `ZngurSpec` was built inside `after_analysis` (while the `TyCtxt` was
                // alive) and is now printed with `IDLPrinter`.
                let Some(output_path) = self.output_path.take() else {
                    eprintln!("autozng: no `.zng` output path was computed");
                    return Err(CompilerError::Failed);
                };
                match render::print_to_file(&spec, &output_path) {
                    Ok(()) => Ok(()),
                    Err(err) => {
                        eprintln!(
                            "autozng: failed to write `{}`: {err}",
                            output_path.display()
                        );
                        Err(CompilerError::Failed)
                    }
                }
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
    F: for<'tcx> FnMut(TyCtxt<'tcx>) -> ControlFlow<AnalyzeReport, ZngurSpec> + Send,
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
                visible_paths: RefCell::new(None),
            };
            set_ctxt(driver_ctx, || {
                self.result = Some((self.analyze_callback)(tcx));

                // The `.zng` destination depends on the crate name, which is only reachable
                // here (inside the `rustc_public` context, while `tcx` is alive).
                let crate_name = rustc_public::local_crate().name.to_string();
                self.output_path = Some(output_path(&self.cfg, &crate_name));
            })
        })
        .unwrap();

        // stop after analysis
        Compilation::Stop
    }
}

/// Decide where the `.zng` file for `crate_name` should be written.
///
/// Precedence: an explicit `--output-zng` (or `AUTOZNG_OUTPUT_ZNG`) path is used verbatim;
/// otherwise the crate-named file goes into `AUTOZNG_OUTPUT_DIR`, then rustc's `--out-dir`, and
/// finally the current directory (rustc's own default). rustc's `-o` argument is intentionally
/// ignored.
fn output_path(config: &Config, crate_name: &str) -> PathBuf {
    if let Some(explicit) = &config.output.output_zng {
        return explicit.clone();
    }

    let dir = config
        .output
        .output_dir
        .clone()
        .or_else(|| config.output.out_dir.clone())
        .unwrap_or_else(|| PathBuf::from("."));
    dir.join(format!("{crate_name}.zng"))
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

pub(crate) fn set_ctxt<R>(ctx: &DriverContext, f: impl FnOnce() -> R) -> R {
    let ctx_ptr: *const () = (&raw const ctx) as _;
    TCX.set(&Cell::new(ctx_ptr), f)
}
