//! Run the signer, listening to the serial port

#![no_std]
#![no_main]
#![feature(alloc_error_handler)]
extern crate alloc;

use alloc::format;
use alloc::string::String;

use cortex_m_rt::entry;

#[allow(unused_imports)]
use log::{debug, info, trace};

use vls_protocol_signer::vls_protocol::msgs::{self, Message};
use vls_protocol_signer::vls_protocol::serde_bolt::WireString;

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
    let (mut delay, timer, mut serial, mut sdio, mut disp) = device::make_devices();

    let mut counter = 0;

    #[cfg(feature = "sdio")]
    {
        sdcard::init_sdio(&mut sdio, &mut delay);

        let mut block = [0u8; 512];

        let res = sdio.read_block(0, &mut block);
        info!("sdcard read result {:?}", res);

        sdcard::test(sdio);
    }

    timer::start(timer);

    loop {
        if counter % 100 == 0 || counter < 100 {
            disp.clear_screen();
            disp.show_text(format!("{}", counter));
        }
        let message = msgs::read(&mut serial).expect("message read failed");
        match message {
            Message::Ping(p) => {
                info!("got ping with {} {}", p.id, String::from_utf8(p.message.0).unwrap());
                let reply =
                    msgs::Pong { id: p.id, message: WireString("pong".as_bytes().to_vec()) };
                msgs::write(&mut serial, reply).expect("message write failed");
            }
            Message::Unknown(u) => {
                panic!("Unknown message type {}", u.message_type);
            }
            _ => {
                panic!("Unhandled message");
            }
        }
        // delay.delay_ms(100u16);
        counter += 1;
    }
}
