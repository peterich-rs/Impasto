//! Perfetto TrackEvent protobuf subset from google/perfetto 1e9c61e98.
//! The Linux CI import fixture checks this schema with the upstream processor.

use prost::Message;

pub(crate) const SEQ_INCREMENTAL_STATE_CLEARED: u32 = 1;
pub(crate) const SEQ_NEEDS_INCREMENTAL_STATE: u32 = 2;

pub(crate) const CHILD_ORDERING_EXPLICIT: i32 = 3;

pub(crate) const COUNTER_UNIT_COUNT: i32 = 2;
pub(crate) const COUNTER_UNIT_SIZE_BYTES: i32 = 3;

pub(crate) const EVENT_TYPE_SLICE_BEGIN: i32 = 1;
pub(crate) const EVENT_TYPE_SLICE_END: i32 = 2;
pub(crate) const EVENT_TYPE_INSTANT: i32 = 3;
pub(crate) const EVENT_TYPE_COUNTER: i32 = 4;

#[cfg(test)]
#[derive(Clone, PartialEq, Message)]
pub(crate) struct Trace {
    #[prost(message, repeated, tag = "1")]
    pub packet: Vec<TracePacket>,
}

#[derive(Clone, PartialEq, Message)]
pub(crate) struct TracePacket {
    #[prost(uint64, optional, tag = "8")]
    pub timestamp: Option<u64>,
    #[prost(message, optional, tag = "11")]
    pub track_event: Option<TrackEvent>,
    #[prost(message, optional, tag = "60")]
    pub track_descriptor: Option<TrackDescriptor>,
    #[prost(uint32, optional, tag = "10")]
    pub trusted_packet_sequence_id: Option<u32>,
    #[prost(message, optional, tag = "12")]
    pub interned_data: Option<InternedData>,
    #[prost(uint32, optional, tag = "13")]
    pub sequence_flags: Option<u32>,
}

#[derive(Clone, PartialEq, Message)]
pub(crate) struct TrackDescriptor {
    #[prost(uint64, optional, tag = "1")]
    pub uuid: Option<u64>,
    #[prost(string, optional, tag = "2")]
    pub name: Option<String>,
    #[prost(message, optional, tag = "3")]
    pub process: Option<ProcessDescriptor>,
    #[prost(message, optional, tag = "4")]
    pub thread: Option<ThreadDescriptor>,
    #[prost(uint64, optional, tag = "5")]
    pub parent_uuid: Option<u64>,
    #[prost(message, optional, tag = "8")]
    pub counter: Option<CounterDescriptor>,
    #[prost(int32, optional, tag = "11")]
    pub child_ordering: Option<i32>,
    #[prost(int32, optional, tag = "12")]
    pub sibling_order_rank: Option<i32>,
}

#[derive(Clone, PartialEq, Message)]
pub(crate) struct ProcessDescriptor {
    #[prost(int32, optional, tag = "1")]
    pub pid: Option<i32>,
    #[prost(string, optional, tag = "6")]
    pub process_name: Option<String>,
}

#[derive(Clone, PartialEq, Message)]
pub(crate) struct ThreadDescriptor {
    #[prost(int32, optional, tag = "1")]
    pub pid: Option<i32>,
    #[prost(int64, optional, tag = "2")]
    pub tid: Option<i64>,
    #[prost(string, optional, tag = "5")]
    pub thread_name: Option<String>,
}

#[derive(Clone, PartialEq, Message)]
pub(crate) struct CounterDescriptor {
    #[prost(int32, optional, tag = "3")]
    pub unit: Option<i32>,
    #[prost(string, optional, tag = "6")]
    pub unit_name: Option<String>,
}

#[derive(Clone, PartialEq, Message)]
pub(crate) struct TrackEvent {
    #[prost(uint64, repeated, packed = "false", tag = "3")]
    pub category_iids: Vec<u64>,
    #[prost(message, repeated, tag = "4")]
    pub debug_annotations: Vec<DebugAnnotation>,
    #[prost(int32, optional, tag = "9")]
    pub r#type: Option<i32>,
    #[prost(uint64, optional, tag = "10")]
    pub name_iid: Option<u64>,
    #[prost(uint64, optional, tag = "11")]
    pub track_uuid: Option<u64>,
    #[prost(int64, optional, tag = "30")]
    pub counter_value: Option<i64>,
    #[prost(uint64, optional, tag = "34")]
    pub source_location_iid: Option<u64>,
    #[prost(double, optional, tag = "44")]
    pub double_counter_value: Option<f64>,
    #[prost(fixed64, repeated, packed = "false", tag = "47")]
    pub flow_ids: Vec<u64>,
    #[prost(fixed64, repeated, packed = "false", tag = "48")]
    pub terminating_flow_ids: Vec<u64>,
}

#[derive(Clone, PartialEq, Message)]
pub(crate) struct DebugAnnotation {
    #[prost(uint64, optional, tag = "1")]
    pub name_iid: Option<u64>,
    #[prost(bool, optional, tag = "2")]
    pub bool_value: Option<bool>,
    #[prost(uint64, optional, tag = "3")]
    pub uint_value: Option<u64>,
    #[prost(int64, optional, tag = "4")]
    pub int_value: Option<i64>,
    #[prost(double, optional, tag = "5")]
    pub double_value: Option<f64>,
    #[prost(string, optional, tag = "6")]
    pub string_value: Option<String>,
}

#[derive(Clone, PartialEq, Message)]
pub(crate) struct InternedData {
    #[prost(message, repeated, tag = "1")]
    pub event_categories: Vec<EventCategory>,
    #[prost(message, repeated, tag = "2")]
    pub event_names: Vec<EventName>,
    #[prost(message, repeated, tag = "3")]
    pub debug_annotation_names: Vec<DebugAnnotationName>,
    #[prost(message, repeated, tag = "4")]
    pub source_locations: Vec<SourceLocation>,
}

#[derive(Clone, PartialEq, Message)]
pub(crate) struct EventCategory {
    #[prost(uint64, optional, tag = "1")]
    pub iid: Option<u64>,
    #[prost(string, optional, tag = "2")]
    pub name: Option<String>,
}

#[derive(Clone, PartialEq, Message)]
pub(crate) struct EventName {
    #[prost(uint64, optional, tag = "1")]
    pub iid: Option<u64>,
    #[prost(string, optional, tag = "2")]
    pub name: Option<String>,
}

#[derive(Clone, PartialEq, Message)]
pub(crate) struct DebugAnnotationName {
    #[prost(uint64, optional, tag = "1")]
    pub iid: Option<u64>,
    #[prost(string, optional, tag = "2")]
    pub name: Option<String>,
}

#[derive(Clone, PartialEq, Message)]
pub(crate) struct SourceLocation {
    #[prost(uint64, optional, tag = "1")]
    pub iid: Option<u64>,
    #[prost(string, optional, tag = "2")]
    pub file_name: Option<String>,
    #[prost(string, optional, tag = "3")]
    pub function_name: Option<String>,
    #[prost(uint32, optional, tag = "4")]
    pub line_number: Option<u32>,
}
