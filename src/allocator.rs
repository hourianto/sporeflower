#[global_allocator]
static GLOBAL: mimalloc::MiMalloc = mimalloc::MiMalloc;

#[allow(unsafe_code)]
pub(super) fn configure() {
    // mimalloc.h's mi_option_purge_delay; the bindings omit this enum member.
    const PURGE_DELAY: libmimalloc_sys::mi_option_t = 15;
    // SAFETY: called once at main entry, before starting worker threads.
    // This changes a valid scalar option; set_default preserves explicit
    // MIMALLOC_PURGE_DELAY (and legacy MIMALLOC_RESET_DELAY) overrides.
    unsafe { libmimalloc_sys::mi_option_set_default(PURGE_DELAY, 10) };
}
