use crate::prelude::*;
use std::fmt::Debug;

use console::{Emoji, Style, StyledObject};
use tracing::{
    field::{Field, Visit},
    metadata::LevelFilter,
    span::Attributes,
    Event, Id, Level, Subscriber,
};
use tracing_subscriber::{
    filter::{EnvFilter, Targets},
    layer::{Context, Layer},
    prelude::*,
    registry::{LookupSpan, SpanRef},
};

use clap::{Args, ValueEnum};

#[derive(Clone, Copy, Debug, ValueEnum)]
enum ColorChoice {
    Auto,
    Always,
    Never,
}

#[derive(Args)]
pub struct OutputArgs {
    /// Increase verbosity. (Can be repeated.)
    #[arg(short, long, action = clap::ArgAction::Count, global = true)]
    verbose: u8,
    /// Reduce verbosity. (Can be repeated.)
    #[arg(short, long, action = clap::ArgAction::Count, global = true)]
    quiet: u8,
    #[arg(long, default_value_t = ColorChoice::Auto, value_enum, value_name = "WHEN", global = true)]
    color: ColorChoice,
}

struct PosyUILayer;

struct WithMessage<'a, F>(&'a F)
where
    F: Fn(&dyn Debug);

impl<'a, F> Visit for WithMessage<'a, F>
where
    F: Fn(&dyn Debug),
{
    fn record_debug(&mut self, field: &Field, value: &dyn Debug) {
        if field.name() == "message" {
            (self.0)(value);
        }
    }
}

struct MessageAsString(String);

const WARNING: Lazy<StyledObject<Emoji<'static, 'static>>> = Lazy::new(|| {
    Style::new()
        .yellow()
        .bold()
        .for_stderr()
        .apply_to(Emoji("⚠️  Warning:", "Warning:"))
});

const ERROR: Lazy<StyledObject<Emoji<'static, 'static>>> = Lazy::new(|| {
    Style::new()
        .red()
        .bold()
        .for_stderr()
        .apply_to(Emoji("🛑  Error:", "Error:"))
});

fn collect_context<S>(leaf: Option<SpanRef<S>>) -> Vec<String>
where
    S: Subscriber + for<'a> LookupSpan<'a>,
{
    if let Some(leaf) = leaf {
        leaf.scope()
            .from_root()
            .filter_map(|span| {
                span.extensions()
                    .get::<MessageAsString>()
                    .map(|m| m.0.clone())
            })
            .collect()
    } else {
        vec![]
    }
}

pub fn current_context() -> Vec<String> {
    tracing::dispatcher::get_default(|dispatch| {
        if let Some(registry) = dispatch.downcast_ref::<tracing_subscriber::Registry>()
        {
            // NB: can't use Span::current_span() here, because that has to re-fetch the
            // current dispatcher, and while we're inside a dispatcher::get_default call
            // we temporarily *own* that dispatcher and the current dispatcher gets set
            // to None instead.
            if let Some(leaf_id) = registry.current_span().id() {
                return collect_context(registry.span(leaf_id));
            }
        }
        vec![]
    })
}

impl<S: Subscriber + for<'a> LookupSpan<'a>> Layer<S> for PosyUILayer {
    /// For every context!(...) span, render the message into a String and stash it
    /// inside the tracing_subscriber registry entry for this Span.
    fn on_new_span(&self, attrs: &Attributes<'_>, id: &Id, ctx: Context<'_, S>) {
        let span = ctx.span(id).expect("span should already exist!");
        if span.metadata().target() == POSY_CONTEXT_TARGET {
            attrs.record(&mut WithMessage(&|msg| {
                let as_string = MessageAsString(format!("{:?}", msg));
                span.extensions_mut().insert(as_string);
            }));
        }
    }

    fn on_event(&self, event: &Event<'_>, _ctx: Context<'_, S>) {
        // let leaf = ctx.event_span(&event);
        // for span_render in collect_context(leaf) {
        //     eprintln!("span: {}", span_render);
        // }
        event.record(&mut WithMessage(&|msg| match *event.metadata().level() {
            Level::ERROR => eprintln!("{} {:?}", &*ERROR, msg),
            Level::WARN => eprintln!("{} {:?}", &*WARNING, msg),
            _ => eprintln!("{:?}", msg),
        }));
    }
}

pub const POSY_CONTEXT_TARGET: &str = "posy::context";
#[macro_export]
macro_rules! context {
    ($($arg:tt)*) => {
        let _guard = tracing::span!(target: "posy::context", tracing::Level::ERROR, "context", $($arg)*).entered();
    }
}

struct PosyEyreHandler {
    context: Vec<String>,
    backtrace: backtrace::Backtrace,
}

impl PosyEyreHandler {
    fn new() -> PosyEyreHandler {
        PosyEyreHandler {
            context: current_context(),
            // we may want to make this dependent on RUST_BACKTRACE or similar
            // at some point, but while we're prototyping backtraces are so useful
            // we'll just capture it unconditionall
            backtrace: backtrace::Backtrace::new_unresolved(),
        }
    }
}

impl eyre::EyreHandler for PosyEyreHandler {
    fn debug(
        &self,
        error: &(dyn std::error::Error + 'static),
        f: &mut core::fmt::Formatter<'_>,
    ) -> core::fmt::Result {
        write!(f, "In context: {:?}: {}", self.context, error)?;
        // clone to make it mutable so we can resolve symbols
        let mut backtrace = self.backtrace.clone();
        backtrace.resolve();
        write!(f, "Backtrace:\n{backtrace:?}")?;
        Ok(())
    }
}

pub fn init(args: &OutputArgs) {
    eyre::set_hook(Box::new(|_| Box::new(PosyEyreHandler::new())))
        .expect("eyre handler already installed?");

    let verbosity = args
        .verbose
        .try_into()
        .unwrap_or(i8::MAX)
        .saturating_sub(args.quiet.try_into().unwrap_or(i8::MAX));

    let global_level = match verbosity {
        2.. => Level::DEBUG,
        1 => Level::TRACE,
        0 => Level::INFO,
        -1 => Level::WARN,
        // https://github.com/rust-lang/rust/issues/67264
        i8::MIN..=-2 => Level::ERROR,
    };

    match args.color {
        ColorChoice::Auto => (),
        ColorChoice::Always => console::set_colors_enabled_stderr(true),
        ColorChoice::Never => console::set_colors_enabled_stderr(false),
    }

    let s = tracing_subscriber::registry()
        .with(PosyUILayer.with_filter(Targets::new().with_target("posy", global_level)))
        .with(
            tracing_subscriber::fmt::layer().with_filter(
                EnvFilter::builder()
                    .with_default_directive(LevelFilter::OFF.into())
                    .with_env_var("POSY_DEBUG")
                    .from_env_lossy(),
            ),
        );
    s.init();
}
