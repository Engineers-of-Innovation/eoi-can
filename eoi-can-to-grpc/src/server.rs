use std::pin::Pin;
use std::sync::Arc;
use std::time::Duration;

use parking_lot::Mutex;
use tokio::sync::mpsc;
use tokio_stream::wrappers::ReceiverStream;
use tokio_stream::Stream;
use tonic::{Request, Response, Status};

use crate::live_state::LiveState;
use crate::pb::eoi::telemetry::v1::telemetry_server::Telemetry;
use crate::pb::eoi::telemetry::v1::{
    GetSnapshotRequest, ListSessionsRequest, ListSessionsResponse, ReplayRequest, Snapshot,
    StreamStateRequest,
};
use crate::proto_map::to_proto;

#[derive(Clone)]
pub struct TelemetrySvc {
    pub state: Arc<Mutex<LiveState>>,
    pub session_id: String,
    pub seq: Arc<Mutex<u64>>,
}

impl TelemetrySvc {
    pub(crate) fn snapshot(&self) -> Snapshot {
        let mut seq = self.seq.lock();
        *seq += 1;
        let view = self.state.lock().view(std::time::Instant::now());
        to_proto(&view, *seq, &self.session_id)
    }
}

#[tonic::async_trait]
impl Telemetry for TelemetrySvc {
    type StreamStateStream =
        Pin<Box<dyn Stream<Item = Result<Snapshot, Status>> + Send + 'static>>;

    async fn stream_state(
        &self,
        request: Request<StreamStateRequest>,
    ) -> Result<Response<Self::StreamStateStream>, Status> {
        let req = request.into_inner();
        let mut hz = req.hz;
        if hz == 0 {
            hz = 5;
        }
        hz = hz.clamp(1, 25);
        let period = Duration::from_secs_f32(1.0 / hz as f32);
        let (tx, rx) = mpsc::channel::<Result<Snapshot, Status>>(4);
        let svc = TelemetrySvc {
            state: self.state.clone(),
            session_id: self.session_id.clone(),
            seq: self.seq.clone(),
        };
        let groups = req.groups;
        tokio::spawn(async move {
            let mut tick = tokio::time::interval(period);
            loop {
                tick.tick().await;
                let mut snap = svc.snapshot();
                filter_groups(&mut snap, &groups);
                if tx.send(Ok(snap)).await.is_err() {
                    break;
                }
            }
        });
        Ok(Response::new(Box::pin(ReceiverStream::new(rx))))
    }

    async fn get_snapshot(
        &self,
        _request: Request<GetSnapshotRequest>,
    ) -> Result<Response<Snapshot>, Status> {
        Ok(Response::new(self.snapshot()))
    }

    async fn list_sessions(
        &self,
        _request: Request<ListSessionsRequest>,
    ) -> Result<Response<ListSessionsResponse>, Status> {
        // The bridge has no archive — only the relay does (eoi-grpc-telemetry).
        Err(Status::unimplemented("the bridge does not archive sessions; ask the relay"))
    }

    type ReplayStream = Pin<Box<dyn Stream<Item = Result<Snapshot, Status>> + Send + 'static>>;

    async fn replay(
        &self,
        _request: Request<ReplayRequest>,
    ) -> Result<Response<Self::ReplayStream>, Status> {
        Err(Status::unimplemented("the bridge does not archive sessions; ask the relay"))
    }
}

fn filter_groups(snap: &mut Snapshot, groups: &[String]) {
    if groups.is_empty() {
        return;
    }
    let want = |name: &str| groups.iter().any(|g| g.eq_ignore_ascii_case(name));
    if !want("power") {
        snap.power = None;
    }
    if !want("battery") {
        snap.battery = None;
    }
    if !want("gnss") {
        snap.gnss = None;
    }
    if !want("motor") {
        snap.motor = None;
    }
    if !want("rudder") {
        snap.rudder = None;
    }
    if !want("throttle") {
        snap.throttle = None;
    }
    if !want("mppt") {
        snap.mppt.clear();
        snap.hottest_mppt = None;
    }
    if !want("height") {
        snap.height.clear();
    }
    if !want("warnings") {
        snap.warnings = None;
    }
    if !want("bus") {
        snap.bus = None;
    }
}
