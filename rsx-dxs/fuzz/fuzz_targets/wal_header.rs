#![no_main]
use libfuzzer_sys::fuzz_target;
use rsx_dxs::WalHeader;

fuzz_target!(|data: &[u8]| {
    // The decoder must not panic on any input.
    let _ = WalHeader::from_bytes(data);
});
