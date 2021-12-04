//! Provide extensions to the [`tracing`] libraries to buffer complete traces
//! and enable tail sampling.
//!
//! Some functionality provided by [`tracing`] is reimplemented in terms of this package, such as
//! the opentelemetry integration. The upstream integration forwards span data as they close rather
//! than buffer, but this prevents sophisticated tail sampling.
//!
//! [`tracing`]: https://github.com/tokio-rs/tracing

use anymap::AnyMap;
use std::collections::HashMap;
use tracing::metadata::Metadata;
use uuid::Uuid;

/// Buffers data and builds complete traces prior to exporting
pub struct Layer {}

pub struct ChainedExporter<E1, E2> {
    a: E1,
    b: E2,
}

impl<E1: Export, E2: Export> Export for ChainedExporter<E1, E2> {
    fn export<'a>(&self, trace: &'a Trace) -> bool {
        if self.a.export(trace) {
            self.b.export(trace)
        }
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

    fn chain<E2: Export>(self, e2: E2) -> ChainedExporter<Self, E2> {
        ChainedExporter { a: self, b: e2 }
    }
}

pub struct KeyValue {
    key: String,
    value: usize, // TODO
}

pub struct SpanId(Uuid);

pub struct Span {
    id: SpanId,
    parent: Option<Uuid>,
    attributes: Vec<KeyValue>,
    metadata: &'static Metadata<'static>,
    extensions: AnyMap,
}

pub struct Trace {
    id: Uuid,
    root: SpanId,
    spans: HashMap<SpanId, Span>,
}
