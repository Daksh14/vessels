use crate::errors::Error;
use crate::network::{
    mesh::{Channel, Negotiation, NegotiationItem, Peer, SessionDescriptionType},
    DataChannel,
};
use crossbeam_channel::{unbounded, Receiver, Sender, TryRecvError};
use futures::{
    future::err, lazy, task::AtomicTask, Async, AsyncSink, Future, Poll, Sink, StartSend, Stream,
};
use std::sync::{
    atomic::{AtomicBool, Ordering},
    Arc,
};
use stdweb::{web::ArrayBuffer, Reference};

struct RTCDataChannelOpening {
    channel: Option<RTCDataChannel>,
    open: Arc<AtomicBool>,
    open_task: Arc<AtomicTask>,
}

impl Future for RTCDataChannelOpening {
    type Item = Box<dyn DataChannel>;
    type Error = Error;

    fn poll(&mut self) -> Poll<Self::Item, Self::Error> {
        if !self.open.load(Ordering::SeqCst) {
            self.open_task.register();
            Ok(Async::NotReady)
        } else {
            let mut channel = None;
            std::mem::swap(&mut self.channel, &mut channel);
            Ok(Async::Ready(Box::new(channel.unwrap())))
        }
    }
}

#[derive(Clone)]
struct RTCDataChannel {
    channel: web_sys::RtcDataChannel,
    data: Receiver<Vec<u8>>,
    task: Arc<AtomicTask>,
}

impl DataChannel for RTCDataChannel {}

impl Stream for RTCDataChannel {
    type Item = Vec<u8>;
    type Error = Error;

    fn poll(&mut self) -> Poll<Option<Self::Item>, Self::Error> {
        match self.data.try_recv() {
            Ok(message) => Ok(Async::Ready(Some(message))),
            Err(err) => match err {
                TryRecvError::Disconnected => {
                    panic!("channel disconnected in channel stream");
                }
                TryRecvError::Empty => {
                    self.task.register();
                    Ok(Async::NotReady)
                }
            },
        }
    }
}

impl Sink for RTCDataChannel {
    type SinkItem = Vec<u8>;
    type SinkError = Error;

    fn start_send(&mut self, item: Self::SinkItem) -> StartSend<Self::SinkItem, Self::SinkError> {
        js! {
            @{&self.channel}.send(new Uint8Array(@{item}));
        };
        Ok(AsyncSink::Ready)
    }
    fn poll_complete(&mut self) -> Poll<(), Self::SinkError> {
        Ok(Async::Ready(()))
    }
}

impl RTCDataChannel {
    fn make_channel(channel: web_sys::RtcDataChannel) -> RTCDataChannel {
        let (sender, data) = unbounded();
        let task = Arc::new(AtomicTask::new());
        let task_cloned = task.clone();
        let handle_message = move |data: ArrayBuffer| {
            let data = Vec::<u8>::from(data);
            sender.send(data).unwrap();
            task_cloned.notify();
        };
        js! {
            @{&channel}.onmessage = (message) => {
                @{handle_message}(message.data);
            };
        }
        RTCDataChannel {
            channel,
            data,
            task,
        }
    }
    fn new(channel: web_sys::RtcDataChannel, sender: Sender<Channel>, add_task: Arc<AtomicTask>) {
        let data_channel = RTCDataChannel::make_channel(channel.clone());
        let on_open = move || {
            sender
                .send(Channel::DataChannel(Box::new(data_channel.clone())))
                .unwrap();
            add_task.notify();
        };
        js! {
            @{&channel}.onopen = () => {
                @{on_open}();
            };
        };
    }
    fn new_local(channel: web_sys::RtcDataChannel) -> RTCDataChannelOpening {
        let open_task = Arc::new(AtomicTask::new());
        let open = Arc::new(AtomicBool::new(false));
        let task = open_task.clone();
        let open_cloned = open.clone();
        let channel_ready = move || {
            open_cloned.store(true, Ordering::SeqCst);
            task.notify();
        };
        js! {
            @{&channel}.onopen = () => {
                @{channel_ready}();
            };
        };
        RTCDataChannelOpening {
            channel: Some(RTCDataChannel::make_channel(channel)),
            open,
            open_task,
        }
    }
}

struct RTCPeer {
    connection: web_sys::RtcPeerConnection,
    channels: Receiver<Channel>,
    task: Arc<AtomicTask>,
}

impl Peer for RTCPeer {
    fn data_channel(&mut self) -> Box<dyn Future<Item = Box<dyn DataChannel>, Error = Error>> {
        Box::new(RTCDataChannel::new_local(
            self.connection.create_data_channel("test"),
        ))
    }
}

impl Stream for RTCPeer {
    type Item = Channel;
    type Error = Error;

    fn poll(&mut self) -> Poll<Option<Self::Item>, Self::Error> {
        match self.channels.try_recv() {
            Ok(channel) => Ok(Async::Ready(Some(channel))),
            Err(err) => match err {
                TryRecvError::Disconnected => {
                    panic!("channel disconnected in channel stream");
                }
                TryRecvError::Empty => {
                    self.task.register();
                    Ok(Async::NotReady)
                }
            },
        }
    }
}

impl RTCPeer {
    fn new(connection: web_sys::RtcPeerConnection) -> RTCPeer {
        let (sender, receiver) = unbounded();
        let task = Arc::new(AtomicTask::new());
        let add_task = task.clone();
        let add_data_channel = move |channel: web_sys::RtcDataChannel| {
            RTCDataChannel::new(channel, sender.clone(), add_task.clone());
        };
        js! {
            @{&connection}.ondatachannel = (e) => {
                @{add_data_channel}(e.channel);
            };
        };
        RTCPeer {
            connection,
            channels: receiver,
            task,
        }
    }
}

struct RTCNegotiation {
    outgoing: Receiver<NegotiationItem>,
    outgoing_sender: Sender<NegotiationItem>,
    outgoing_task: Arc<AtomicTask>,
    connection: web_sys::RtcPeerConnection,
}

impl Negotiation for RTCNegotiation {}

impl Stream for RTCNegotiation {
    type Item = NegotiationItem;
    type Error = Error;

    fn poll(&mut self) -> Poll<Option<Self::Item>, Self::Error> {
        match self.outgoing.try_recv() {
            Ok(negotiation) => Ok(Async::Ready(Some(negotiation))),
            Err(err) => match err {
                TryRecvError::Disconnected => {
                    panic!("channel disconnected in negotiation stream");
                }
                TryRecvError::Empty => {
                    self.outgoing_task.register();
                    Ok(Async::NotReady)
                }
            },
        }
    }
}

impl Sink for RTCNegotiation {
    type SinkItem = NegotiationItem;
    type SinkError = Error;

    fn start_send(&mut self, item: Self::SinkItem) -> StartSend<Self::SinkItem, Self::SinkError> {
        self.handle_incoming(item);
        Ok(AsyncSink::Ready)
    }
    fn poll_complete(&mut self) -> Poll<(), Self::SinkError> {
        Ok(Async::Ready(()))
    }
}

impl RTCNegotiation {
    fn new(connection: web_sys::RtcPeerConnection) -> RTCNegotiation {
        let (outgoing_sender, outgoing_receiver) = unbounded();
        let outgoing_task = Arc::new(AtomicTask::new());
        let outgoing_task_cloned = outgoing_task.clone();
        let outgoing_sender_cloned = outgoing_sender.clone();
        let n_connection = connection.clone();
        let negotiate = move || {
            let outgoing_task_cloned = outgoing_task_cloned.clone();
            let outgoing_sender_cloned = outgoing_sender_cloned.clone();
            let send_offer = move |sdp: String| {
                outgoing_sender_cloned
                    .send(NegotiationItem::SessionDescription(
                        SessionDescriptionType::Offer,
                        sdp,
                    ))
                    .expect("could not send offer");
                outgoing_task_cloned.notify();
            };
            js! {
                let connection = @{n_connection.clone()};
                connection.createOffer().catch((error) => {
                    console.log(error);
                }).then((offer) => {
                    connection.setLocalDescription(offer).catch((error) => {
                        console.log(error);
                    }).then(() => {
                        @{send_offer}(offer.sdp);
                    });
                });
            };
        };
        let ice_sender = outgoing_sender.clone();
        let ice_task = outgoing_task.clone();
        let send_candidate = move |candidate: String, _ufrag: String| {
            ice_sender
                .send(NegotiationItem::ConnectivityEstablishmentCandidate(Some(
                    candidate,
                )))
                .expect("could not send candidate");
            ice_task.notify();
        };
        let ice_termination_sender = outgoing_sender.clone();
        let ice_termination_task = outgoing_task.clone();
        let send_candidate_termination = move || {
            ice_termination_sender
                .send(NegotiationItem::ConnectivityEstablishmentCandidate(None))
                .unwrap();
            ice_termination_task.notify();
        };
        js! {
            @{&connection}.onicecandidate = (e) => {
                if (!e.candidate) {
                    @{send_candidate_termination}();
                    return;
                };
                @{send_candidate}(e.candidate.candidate, e.candidate.usernameFragment);
            };
            @{&connection}.onnegotiationneeded = () => {
                @{negotiate}();
            };
        }
        RTCNegotiation {
            outgoing: outgoing_receiver,
            outgoing_sender,
            outgoing_task,
            connection,
        }
    }
    fn create_offer(&mut self) {}
    fn handle_incoming(&mut self, incoming: NegotiationItem) {
        match incoming {
            NegotiationItem::SessionDescription(ty, sdp) => {
                self.handle_session_description(ty, sdp);
            }
            NegotiationItem::ConnectivityEstablishmentCandidate(candidate) => {
                self.handle_connectivity_establishment_candidate(candidate)
            }
        };
    }
    fn handle_connectivity_establishment_candidate(&mut self, candidate: Option<String>) {
        match &candidate {
            Some(candidate) => js! {
                @{&self.connection}.addIceCandidate({
                    candidate: @{&candidate},
                    sdpMid: "0",
                    sdpMLineIndex: 0,
                    usernameFragment: "",
                });
            },
            None => js! {
                //@{&self.connection}.addIceCandidate(null);
            },
        };
    }
    fn handle_session_description(&mut self, ty: SessionDescriptionType, sdp: String) {
        let outgoing_task = self.outgoing_task.clone();
        let outgoing_sender = self.outgoing_sender.clone();
        let connection = self.connection.clone();
        let finish_handle = move || match ty {
            SessionDescriptionType::Offer => {
                let outgoing_sender = outgoing_sender.clone();
                let outgoing_task = outgoing_task.clone();
                let connection = connection.clone();
                let send_answer = move |sdp: String| {
                    outgoing_sender
                        .send(NegotiationItem::SessionDescription(
                            SessionDescriptionType::Answer,
                            sdp,
                        ))
                        .expect("could not send offer");
                    outgoing_task.notify();
                };
                js! {
                    let connection = @{connection};
                    connection.createAnswer().catch((error) => console.log(error)).then((answer) => {
                        connection.setLocalDescription(answer).catch((error) => {
                            console.log(error);
                        }).then(() => @{send_answer}(answer.sdp));
                    });
                }
            }
            SessionDescriptionType::Answer => {}
            SessionDescriptionType::Rollback => {}
        };
        js! {
            @{&self.connection}.setRemoteDescription(new RTCSessionDescription({sdp: @{sdp}, type: @{match ty {
                SessionDescriptionType::Answer => "answer",
                SessionDescriptionType::Offer => "offer",
                SessionDescriptionType::Rollback => "rollback"
            }}})).catch((error) => {
                console.log(error);
            }).then(() => {
                @{finish_handle}();
            });
        };
    }
}

pub(crate) fn new(
) -> impl Future<Item = (Box<dyn Peer + 'static>, Box<dyn Negotiation + 'static>), Error = Error> {
    lazy(move || {
        let connection =
            web_sys::RtcPeerConnection::new().expect("Could not instantiate peer connection");
        let peer: Box<dyn Peer> = Box::new(RTCPeer::new(connection.clone()));
        let negotiation: Box<dyn Negotiation> = Box::new(RTCNegotiation::new(connection));
        Ok((peer, negotiation))
    })
}
