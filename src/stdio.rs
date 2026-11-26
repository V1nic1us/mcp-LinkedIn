use std::{
    collections::HashMap,
    io::{self, BufRead, Read, Write},
    sync::{mpsc as std_mpsc, Arc},
    thread,
    time::Duration,
};

use async_trait::async_trait;
use rust_mcp_sdk::{
    schema::{
        ClientMessage, ClientMessages, MessageFromServer, RequestId, Result as McpResult,
        ResultFromServer, SdkError, ServerMessage, ServerMessages,
    },
    IoStream, McpDispatch, MessageDispatcher, Transport, TransportDispatcher, TransportError,
    TransportOptions, TransportResult,
};
use tokio::{
    sync::{mpsc, oneshot, Mutex, RwLock, RwLockReadGuard},
    task::{AbortHandle, JoinHandle},
};
use tokio_stream::wrappers::ReceiverStream;

const INPUT_DIAGNOSTIC_CAPACITY: usize = 16;

pub(crate) enum StdioFailure {
    Read,
    Write(io::ErrorKind),
}

type PendingRequests = Arc<Mutex<HashMap<RequestId, oneshot::Sender<ClientMessage>>>>;
type Dispatcher = Arc<RwLock<Option<MessageDispatcher<ClientMessage>>>>;

pub(crate) struct StdioTransport {
    incoming: Mutex<Option<ReceiverStream<ClientMessages>>>,
    relay: AbortHandle,
    dispatcher: Dispatcher,
    pending: PendingRequests,
    stderr: RwLock<Option<IoStream>>,
}

impl StdioTransport {
    pub(crate) fn open() -> io::Result<(Self, mpsc::UnboundedReceiver<StdioFailure>)> {
        let options = TransportOptions::default();
        let (failure_tx, failures) = mpsc::unbounded_channel();
        let (read_tx, mut read_rx) = mpsc::channel(1);
        let (incoming_tx, incoming_rx) = mpsc::channel(options.channel_capacity);
        let (write_tx, mut write_rx) = mpsc::channel::<(
            String,
            oneshot::Sender<TransportResult<()>>,
        )>(options.channel_capacity);

        // Use dedicated threads for blocking stdio. Tokio can shut down even
        // if the peer leaves a pipe open while the other pipe fails.
        let write_failure = failure_tx.clone();
        thread::Builder::new()
            .name("mcp-stdout".into())
            .spawn(move || {
                let mut stdout = io::stdout().lock();
                while let Some((frame, ack)) = write_rx.blocking_recv() {
                    let result = writeln!(stdout, "{frame}").and_then(|()| stdout.flush());
                    if let Err(error) = result {
                        let _ = write_failure.send(StdioFailure::Write(error.kind()));
                        let _ = ack.send(Err(TransportError::Io(error)));
                        break;
                    }
                    let _ = ack.send(Ok(()));
                }
            })?;

        let (diagnostics, diagnostic_rx) =
            std_mpsc::sync_channel::<&'static str>(INPUT_DIAGNOSTIC_CAPACITY);
        thread::Builder::new()
            .name("mcp-input-diagnostics".into())
            .spawn(move || {
                for message in diagnostic_rx {
                    if writeln!(io::stderr().lock(), "linkedin-mcp: {message}").is_err() {
                        break;
                    }
                }
            })?;

        thread::Builder::new()
            .name("mcp-stdin".into())
            .spawn(move || {
                if read_frames(
                    io::stdin().lock(),
                    &read_tx,
                    &diagnostics,
                    options.max_line_length,
                )
                .is_err()
                {
                    let _ = failure_tx.send(StdioFailure::Read);
                }
            })?;

        // Aborting the relay closes the SDK stream without waiting on an OS read.
        let relay = tokio::spawn(async move {
            while let Some(frame) = read_rx.recv().await {
                if incoming_tx.send(frame).await.is_err() {
                    break;
                }
            }
        })
        .abort_handle();

        let pending = PendingRequests::default();
        let dispatcher =
            MessageDispatcher::new_with_acknowledgement(pending.clone(), write_tx, options.timeout);
        Ok((
            Self {
                incoming: Mutex::new(Some(ReceiverStream::new(incoming_rx))),
                relay,
                dispatcher: Arc::new(RwLock::new(Some(dispatcher))),
                pending,
                stderr: RwLock::new(Some(IoStream::Writable(Box::pin(tokio::io::stderr())))),
            },
            failures,
        ))
    }

    async fn dispatcher(
        &self,
    ) -> TransportResult<RwLockReadGuard<'_, MessageDispatcher<ClientMessage>>> {
        RwLockReadGuard::try_map(self.dispatcher.read().await, Option::as_ref)
            .map_err(|_| SdkError::connection_closed().into())
    }
}

fn read_frames(
    mut input: impl BufRead,
    sender: &mpsc::Sender<ClientMessages>,
    diagnostics: &std_mpsc::SyncSender<&'static str>,
    max_length: usize,
) -> io::Result<()> {
    let mut frame = Vec::new();
    loop {
        frame.clear();
        let limit = max_length as u64 + 1;
        if input.by_ref().take(limit).read_until(b'\n', &mut frame)? == 0 {
            return Ok(());
        }
        if frame.len() > max_length && !frame.ends_with(b"\n") {
            // Drop the diagnostic if stderr cannot keep up.
            let _ = diagnostics.try_send("discarded oversized MCP frame on stdin");
            input.skip_until(b'\n')?;
            continue;
        }
        match serde_json::from_slice::<ClientMessages>(&frame) {
            Ok(message) => {
                if sender.blocking_send(message).is_err() {
                    return Ok(());
                }
            }
            Err(_) => {
                let _ = diagnostics.try_send("discarded malformed MCP frame on stdin");
            }
        }
    }
}

#[async_trait]
impl Transport<ClientMessages, MessageFromServer, ClientMessage, ServerMessages, ServerMessage>
    for StdioTransport
{
    async fn start(&self) -> TransportResult<ReceiverStream<ClientMessages>> {
        self.incoming
            .lock()
            .await
            .take()
            .ok_or_else(|| SdkError::connection_closed().into())
    }

    fn message_sender(&self) -> Dispatcher {
        self.dispatcher.clone()
    }

    fn error_stream(&self) -> &RwLock<Option<IoStream>> {
        &self.stderr
    }

    async fn shut_down(&self) -> TransportResult<()> {
        self.relay.abort();
        self.pending.lock().await.clear();
        // An in-flight response may hold the dispatcher while stdout blocks.
        if let Ok(mut dispatcher) = self.dispatcher.try_write() {
            dispatcher.take();
        }
        Ok(())
    }

    async fn is_shut_down(&self) -> bool {
        self.relay.is_finished()
    }

    async fn consume_string_payload(&self, _: &str) -> TransportResult<()> {
        Err(TransportError::Internal(
            "Stdio only accepts input from stdin".into(),
        ))
    }

    async fn pending_request_tx(
        &self,
        request_id: &RequestId,
    ) -> Option<oneshot::Sender<ClientMessage>> {
        self.pending.lock().await.remove(request_id)
    }

    async fn keep_alive(
        &self,
        _: Duration,
        _: oneshot::Sender<()>,
    ) -> TransportResult<JoinHandle<()>> {
        Err(TransportError::Internal(
            "Stdio does not use a transport keep-alive".into(),
        ))
    }
}

// The pinned SDK's ToolInputSchema omits additionalProperties. Keep its
// typed list result until dispatch, then preserve the keyword on the wire.
fn close_tool_input_schemas(message: &mut ServerMessage) -> TransportResult<()> {
    let ServerMessage::Response(response) = message else {
        return Ok(());
    };
    let ResultFromServer::ListToolsResult(list) = &response.result else {
        return Ok(());
    };
    let schema_error = || TransportError::Internal("could not serialize MCP tool schemas".into());
    let serde_json::Value::Object(mut fields) =
        serde_json::to_value(list).map_err(|_| schema_error())?
    else {
        return Err(schema_error());
    };
    let tools = fields
        .get_mut("tools")
        .and_then(serde_json::Value::as_array_mut)
        .ok_or_else(schema_error)?;
    for tool in tools {
        let input = tool
            .get_mut("inputSchema")
            .and_then(serde_json::Value::as_object_mut)
            .ok_or_else(schema_error)?;
        input.insert(
            "additionalProperties".into(),
            serde_json::Value::Bool(false),
        );
    }
    response.result = ResultFromServer::Result(McpResult {
        meta: None,
        extra: Some(fields),
    });
    Ok(())
}

#[async_trait]
impl McpDispatch<ClientMessages, ServerMessages, ClientMessage, ServerMessage> for StdioTransport {
    async fn send_message(
        &self,
        mut message: ServerMessages,
        timeout: Option<Duration>,
    ) -> TransportResult<Option<ClientMessages>> {
        match &mut message {
            ServerMessages::Single(message) => close_tool_input_schemas(message)?,
            ServerMessages::Batch(messages) => {
                for message in messages {
                    close_tool_input_schemas(message)?;
                }
            }
        }
        self.dispatcher()
            .await?
            .send_message(message, timeout)
            .await
    }

    async fn send(
        &self,
        message: ServerMessage,
        timeout: Option<Duration>,
    ) -> TransportResult<Option<ClientMessage>> {
        self.dispatcher().await?.send(message, timeout).await
    }

    async fn send_batch(
        &self,
        messages: Vec<ServerMessage>,
        timeout: Option<Duration>,
    ) -> TransportResult<Option<Vec<ClientMessage>>> {
        self.dispatcher().await?.send_batch(messages, timeout).await
    }

    async fn write_str(&self, payload: &str, skip_store: bool) -> TransportResult<()> {
        self.dispatcher()
            .await?
            .write_str(payload, skip_store)
            .await
    }
}

impl
    TransportDispatcher<
        ClientMessages,
        MessageFromServer,
        ClientMessage,
        ServerMessages,
        ServerMessage,
    > for StdioTransport
{
}
