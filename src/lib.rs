//! Provide extensions to the [`tracing`] libraries to buffer complete traces
//! and enable tail sampling.
//!
//! Some functionality provided by [`tracing`] is reimplemented in terms of this package, such as
//! the opentelemetry integration. The upstream integration forwards span data as they close rather
//! than buffer, but this prevents sophisticated tail sampling.
//!
//! [`tracing`]: https://github.com/tokio-rs/tracing

use std::collections::HashMap;
use std::sync::RwLock;
use tracing::metadata::Metadata;
use uuid::Uuid;

use tracing::subscriber::Subscriber;
use tracing::{span, Event};
use tracing_subscriber::layer::Context;
use tracing_subscriber::registry::LookupSpan;

mod extensions;
use extensions::{Extensions, ExtensionsInner, ExtensionsMut};

pub struct ChainedExporter<E1, E2> {
    a: E1,
    b: E2,
}

impl<E1: Export, E2: Export> Export for ChainedExporter<E1, E2> {
    fn export<'a>(&self, trace: &'a Trace) -> bool {
        self.a.export(trace) && self.b.export(trace)
    }
}

/// Export a trace
///
/// Goal is that multiple exports can be chained together and act as global
/// filters to exports further down the chain.
pub trait Export {
    /// Export or filter the trace
    ///
    /// Returning false here will prevent downstream exporters from seeing this
    /// trace.
    fn export<'a>(&self, trace: &'a Trace) -> bool;

    fn chain<E2: Export>(self, e2: E2) -> ChainedExporter<Self, E2>
    where
        Self: Sized,
    {
        ChainedExporter { a: self, b: e2 }
    }
}

#[derive(Debug)]
pub struct KeyValue {
    key: String,
    value: usize, // TODO
}

#[derive(Debug)]
pub struct SpanId(Uuid);

#[derive(Debug)]
pub struct Span {
    id: SpanId,
    parent: Option<Uuid>,
    attributes: Vec<KeyValue>,
    metadata: &'static Metadata<'static>,
    extensions: AnyMap,
}

/// Buffers data and builds complete traces prior to exporting
#[derive(Default, Debug)]
pub struct TailSampleLayer<S> {
    _registry: std::marker::PhantomData<S>,
}

pub struct TraceContext {
    span_id: Uuid,
    trace_id: Uuid,
    parent_id: Option<Uuid>,
    trace: Arc<Trace>,
}

#[derive(Debug)]
pub struct Trace {
    id: Uuid,
    ext: RwLock<ExtensionsInner>,
}

impl Trace {
    fn extensions(&self) -> Extensions<'_> {
        Extensions::new(self.inner.extensions.read().expect("Mutex poisoned"))
    }

    fn extensions_mut(&self) -> ExtensionsMut<'_> {
        ExtensionsMut::new(self.inner.extensions.write().expect("Mutex poisoned"))
    }
}

impl TraceContext {
    pub fn new() -> Self {
        Self {
            span_id: Uuid::new_v4(),
            trace_id: Uuid::new_v4(),
            parent_id: None,
        }
    }

    pub fn child(&self) -> Self {
        TraceContext {
            span_id: Uuid::new_v4(),
            trace_id: self.trace_id,
            parent_id: Some(self.span_id),
        }
    }
}

impl<S> tracing_subscriber::Layer<S> for TailSampleLayer<S>
where
    S: Subscriber + for<'span> LookupSpan<'span>,
    Self: 'static,
{
    /// Notifies this layer that a new span was constructed with the given
    /// `Attributes` and `Id`.
    fn on_new_span(&self, attrs: &span::Attributes<'_>, id: &span::Id, ctx: Context<'_, S>) {
        let span = ctx.span(id).expect("Span not found, this is a bug");
        let mut extensions = span.extensions_mut();
        let parent = self.parent_span(attrs, &ctx);

        let trace_context = parent
            .and_then(|parent_id| {
                let parent = ctx.span(&parent_id).expect("Span not found, this is a bug");
                let parent_ext = parent.extensions();
                parent_ext.get::<TraceContext>().map(|p| p.child())
            })
            .unwrap_or_else(TraceContext::new);

        extensions.insert(trace_context);

        // if parent is none, create a new trace context and insert it into the
        // extensions. If there is a parent, use the parent's trace ID to create
        // the trace context.

        println!(
            "on_new_span {:?}, parent={:?}, {:#?}",
            id,
            self.parent_span(attrs, &ctx),
            attrs
        );
    }

    /// Notifies this layer that a span with the given `Id` recorded the given
    /// `values`.
    // Note: it's unclear to me why we'd need the current span in `record` (the
    // only thing the `Context` type currently provides), but passing it in anyway
    // seems like a good future-proofing measure as it may grow other methods later...
    fn on_record(&self, span: &span::Id, _values: &span::Record<'_>, _ctx: Context<'_, S>) {
        println!("on_record {:?}", span);
    }

    /// Notifies this layer that a span with the ID `span` recorded that it
    /// follows from the span with the ID `follows`.
    // Note: it's unclear to me why we'd need the current span in `record` (the
    // only thing the `Context` type currently provides), but passing it in anyway
    // seems like a good future-proofing measure as it may grow other methods later...
    fn on_follows_from(&self, _span: &span::Id, _follows: &span::Id, _ctx: Context<'_, S>) {}

    /// Notifies this layer that an event has occurred.
    fn on_event(&self, event: &Event<'_>, _ctx: Context<'_, S>) {
        println!("on_event: {:?}", event);
    }

    /// Notifies this layer that a span with the given ID was entered.
    fn on_enter(&self, _id: &span::Id, _ctx: Context<'_, S>) {}

    /// Notifies this layer that the span with the given ID was exited.
    fn on_exit(&self, _id: &span::Id, _ctx: Context<'_, S>) {}

    /// Notifies this layer that the span with the given ID has been closed.
    fn on_close(&self, _id: span::Id, _ctx: Context<'_, S>) {}
}

impl<S> TailSampleLayer<S>
where
    S: Subscriber + for<'span> LookupSpan<'span>,
    Self: 'static,
{
    fn parent_span(&self, attrs: &span::Attributes<'_>, ctx: &Context<'_, S>) -> Option<span::Id> {
        if let Some(parent) = attrs.parent() {
            Some(parent.clone())
        } else if attrs.is_contextual() {
            ctx.lookup_current().map(|s| s.id())
        } else {
            None
        }
    }

    fn new_trace_context(&self) -> TraceContext {
        let context = TraceContext::new();
        let rc = 1;
        {
            let trace_refs = trace_refs.lock.unwrap();
            trace_refs.insert(context.trace_id, rc);
        }
        context
    }

    fn report(&self, trace: &Trace) {
        println!("{:#?}", trace);
    }
}

#[cfg(test)]
mod tests {
    use tracing_subscriber::prelude::*;
    use tracing_subscriber::Registry;

    use super::*;

    #[test]
    fn it_works() {
        let subscriber = Registry::default().with(TailSampleLayer::default());

        tracing::subscriber::set_global_default(subscriber).unwrap();

        tracing::info_span!("base_span").in_scope(|| {
            tracing::info_span!("nested_span").in_scope(|| {
                tracing::info!("event");
            })
        });
    }
}
