//! Provide extensions to the [`tracing`] libraries to buffer complete traces
//! and enable tail sampling.
//!
//! Some functionality provided by [`tracing`] is reimplemented in terms of this package, such as
//! the opentelemetry integration. The upstream integration forwards span data as they close rather
//! than buffer, but this prevents sophisticated tail sampling.
//!
//! [`tracing`]: https://github.com/tokio-rs/tracing

use std::sync::{Arc, RwLock};
use uuid::Uuid;

use tracing::span;
use tracing::subscriber::Subscriber;
use tracing_subscriber::layer::Context;
use tracing_subscriber::registry::LookupSpan;

mod extensions;
use extensions::{Extensions, ExtensionsInner, ExtensionsMut};

pub mod opentelemetry;

/// Buffers data and builds complete traces prior to exporting
#[derive(Default, Debug)]
pub struct TraceContextLayer<S> {
    _registry: std::marker::PhantomData<S>,
}

pub struct TraceContext {
    pub span_id: Uuid,
    pub parent_id: Option<Uuid>,
    pub trace: Trace,
    _hidden: (),
}

#[derive(Debug)]
pub struct TraceInner {
    id: Uuid,
    ext: RwLock<ExtensionsInner>,
}

#[derive(Clone)]
pub struct Trace {
    inner: Arc<TraceInner>,
}

impl Trace {
    fn new() -> Self {
        Trace {
            inner: Arc::new(TraceInner {
                id: Uuid::new_v4(),
                ext: RwLock::new(ExtensionsInner::new()),
            }),
        }
    }

    pub fn id(&self) -> &Uuid {
        &self.inner.id
    }

    pub fn extensions(&self) -> Extensions<'_> {
        Extensions::new(self.inner.ext.read().expect("Mutex poisoned"))
    }

    pub fn extensions_mut(&self) -> ExtensionsMut<'_> {
        ExtensionsMut::new(self.inner.ext.write().expect("Mutex poisoned"))
    }
}

pub struct SampleDecision {
    record_trace: bool,
}

impl TraceContext {
    fn new() -> Self {
        Self {
            span_id: Uuid::new_v4(),
            parent_id: None,
            trace: Trace::new(),
            _hidden: (),
        }
    }

    fn child(&self) -> Self {
        TraceContext {
            span_id: Uuid::new_v4(),
            parent_id: Some(self.span_id),
            trace: self.trace.clone(),
            _hidden: (),
        }
    }
}

impl<S> tracing_subscriber::Layer<S> for TraceContextLayer<S>
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
    }
}

impl<S> TraceContextLayer<S>
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
}

#[cfg(test)]
mod tests {
    use tracing::dispatcher;
    use tracing_subscriber::prelude::*;
    use tracing_subscriber::registry::LookupSpan;
    use tracing_subscriber::Registry;

    use super::*;

    #[test]
    fn it_works() {
        let subscriber = Registry::default().with(TraceContextLayer::default());

        tracing::subscriber::set_global_default(subscriber).unwrap();

        tracing::info_span!("base_span").in_scope(|| {
            let mut root = Uuid::nil();
            dispatcher::get_default(|d| {
                let registry = d.downcast_ref::<Registry>().unwrap();
                let span = d.current_span();
                let spanref = registry.span(&span.id().unwrap()).unwrap();
                let extensions = spanref.extensions();
                let trace_context = extensions.get::<TraceContext>().unwrap();
                assert!(trace_context.parent_id.is_none());
                root = trace_context.span_id;
                trace_context.trace.extensions_mut().insert(42usize);
            });

            tracing::info_span!("nested_span").in_scope(|| {
                dispatcher::get_default(|d| {
                    let registry = d.downcast_ref::<Registry>().unwrap();
                    let span = d.current_span();
                    let spanref = registry.span(&span.id().unwrap()).unwrap();
                    let extensions = spanref.extensions();
                    let trace_context = extensions.get::<TraceContext>().unwrap();
                    assert_eq!(trace_context.parent_id, Some(root));
                    let trace_ext = trace_context.trace.extensions();
                    let trace_data = trace_ext.get::<usize>().unwrap();
                    assert_eq!(*trace_data, 42usize);
                });
            })
        });
    }
}
