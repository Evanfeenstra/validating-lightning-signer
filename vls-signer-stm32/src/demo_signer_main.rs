//! Run the signer, listening to the serial port

#![no_std]
#![no_main]
#![feature(alloc_error_handler)]
extern crate alloc;

use alloc::format;
use alloc::string::String;
use alloc::vec::Vec;

use cortex_m_rt::entry;

#[allow(unused_imports)]
use log::{debug, info, trace};

use device::heap_bytes_used;
use lightning_signer::persist::{DummyPersister, Persist};
use lightning_signer::Arc;
use vls_protocol::model::PubKey;
use vls_protocol::msgs::{self, read_serial_request_header, write_serial_response_header, Message};
use vls_protocol::serde_bolt::WireString;
use vls_protocol_signer::handler::{Handler, RootHandler};
use vls_protocol_signer::lightning_signer;
use vls_protocol_signer::vls_protocol;

mod device;
mod logger;
#[cfg(feature = "sdio")]
mod sdcard;
mod timer;
mod usbserial;

#[entry]
fn main() -> ! {
    logger::init().expect("logger");

    device::init_allocator();

    #[allow(unused)]
    let (mut delay, timer1, timer2, mut serial, mut sdio, mut disp) = device::make_devices();

    logger::set_timer(timer1.clone());

    #[cfg(feature = "sdio")]
    {
        sdcard::init_sdio(&mut sdio, &mut delay);

        let mut block = [0u8; 512];

        let res = sdio.read_block(0, &mut block);
        info!("sdcard read result {:?}", res);

        sdcard::test(sdio);
    }

    timer::start_tim2_interrupt(timer2);

    disp.clear_screen();
    disp.show_text("init");

    let persister: Arc<dyn Persist> = Arc::new(DummyPersister);
    let (sequence, dbid) = read_serial_request_header(&mut serial).expect("read init header");
    assert_eq!(dbid, 0);
    assert_eq!(sequence, 0);
    let init: msgs::HsmdInit2 =
        msgs::read_message(&mut serial).expect("failed to read init message");
    info!("init {:?}", init);
    let allowlist = init.dev_allowlist.iter().map(|s| from_wire_string(s)).collect::<Vec<_>>();
    let seed_opt = init.dev_seed.as_ref().map(|s| s.0);
    let root_handler = RootHandler::new(0, seed_opt, persister, allowlist);
    let init_reply = root_handler.handle(Message::HsmdInit2(init)).expect("handle init");
    write_serial_response_header(&mut serial, sequence).expect("write init header");
    msgs::write_vec(&mut serial, init_reply.as_vec()).expect("write init reply");

    info!("used {} bytes", heap_bytes_used());

    // HACK - use a dummy peer_id until it is plumbed
    let dummy_peer = PubKey([0; 33]);
    loop {
        let (sequence, dbid) =
            read_serial_request_header(&mut serial).expect("read request header");
        let mut message = msgs::read(&mut serial).expect("message read failed");

        // Override the peerid when it is passed in certain messages
        match message {
            Message::NewChannel(ref mut m) => m.node_id = dummy_peer.clone(),
            Message::ClientHsmFd(ref mut m) => m.peer_id = dummy_peer.clone(),
            Message::GetChannelBasepoints(ref mut m) => m.node_id = dummy_peer.clone(),
            Message::SignCommitmentTx(ref mut m) => m.peer_id = dummy_peer.clone(),
            _ => {}
        };

        disp.clear_screen();
        let mut message_d = format!("{:?}", message);
        message_d.truncate(20);
        disp.show_texts(&[format!("req # {}", sequence), message_d.clone()]);
        let start = timer1.now();
        let reply = if dbid > 0 {
            let handler = root_handler.for_new_client(0, dummy_peer.clone(), dbid);
            handler.handle(message).expect("handle")
        } else {
            root_handler.handle(message).expect("handle")
        };
        let end = timer1.now();
        let duration = end.checked_duration_since(start).map(|d| d.to_millis()).unwrap_or(0);
        info!("handled {} in {} ms", message_d, duration);
        write_serial_response_header(&mut serial, sequence).expect("write reply header");
        msgs::write_vec(&mut serial, reply.as_vec()).expect("write reply");
    }
}

fn from_wire_string(s: &WireString) -> String {
    String::from_utf8(s.0.to_vec()).expect("malformed string")
}
