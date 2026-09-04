//! Scarlet Native services required by the bundled C streaming core.

use std::alloc::{Layout, alloc, dealloc};
use std::cell::Cell;
use std::ffi::{CStr, c_char, c_int, c_void};
use std::net::{Ipv4Addr, SocketAddr, ToSocketAddrs};
use std::ptr;
use std::str::FromStr;
use std::sync::atomic::{AtomicU32, AtomicU64, Ordering};
use std::thread;
use std::time::Duration;

use scarlet_sys::{
    GET_RANDOM_FLAG_REQUIRE_ENTROPY, SCTL_SOCKET_SET_NONBLOCK, SCTL_SOCKET_SET_READ_TIMEOUT_MS,
    SCTL_SOCKET_SET_WRITE_TIMEOUT_MS, SCTL_SOCKET_TAKE_ERROR, Syscall, syscall0, syscall1,
    syscall2, syscall3, syscall4,
};

const AF_UNSPEC: c_int = 0;
const AF_INET: c_int = 2;
const SOL_SOCKET: c_int = 1;
const SO_REUSEADDR: c_int = 2;
const SO_ERROR: c_int = 4;
const SO_BROADCAST: c_int = 6;
const SO_SNDBUF: c_int = 7;
const SO_RCVBUF: c_int = 8;
const SO_RCVTIMEO: c_int = 20;
const SO_SNDTIMEO: c_int = 21;
const SO_NONBLOCK: c_int = 0x1001;
const IPPROTO_IP: c_int = 0;
const IP_TTL: c_int = 2;

const EIO: c_int = 5;
const ENOMEM: c_int = 12;
const EINVAL: c_int = 22;
const EMSGSIZE: c_int = 90;
const EPROTONOSUPPORT: c_int = 93;
const EOPNOTSUPP: c_int = 95;
const EAFNOSUPPORT: c_int = 97;

const EAI_NONAME: c_int = -2;
const EAI_FAIL: c_int = -4;

const MUTEX_UNLOCKED: u32 = 0;
const MUTEX_LOCKED: u32 = 1;
const MUTEX_CONTENDED: u32 = 2;
const WAIT_FOREVER: usize = usize::MAX;

thread_local! {
    static ERRNO: Cell<c_int> = const { Cell::new(0) };
}

#[repr(C)]
struct SockAddr {
    family: u16,
    data: [u8; 14],
}

#[repr(C)]
struct SockAddrIn {
    family: u16,
    port: u16,
    address: u32,
    zero: [u8; 8],
}

#[repr(C)]
struct TimeVal {
    seconds: i64,
    microseconds: i64,
}

#[repr(C)]
struct PollFd {
    descriptor: c_int,
    events: i16,
    returned_events: i16,
}

#[repr(C)]
struct PollOptions {
    timeout_ns: i64,
    min_timeout_ns: u64,
}

#[repr(C)]
struct NativeInet4Address {
    address: [u8; 4],
    port: u16,
}

#[repr(C)]
struct AddrInfo {
    flags: c_int,
    family: c_int,
    socket_type: c_int,
    protocol: c_int,
    address_length: u32,
    canonical_name: *mut c_char,
    address: *mut SockAddr,
    next: *mut AddrInfo,
}

#[repr(C)]
struct AddrInfoNode {
    info: AddrInfo,
    address: SockAddrIn,
}

#[repr(C, align(16))]
struct AllocationHeader {
    base: *mut u8,
    layout_size: usize,
    layout_align: usize,
    requested_size: usize,
    magic: usize,
}

const ALLOCATION_MAGIC: usize = 0x5343_4152_4c45_544d;

struct RawMutex {
    state: AtomicU32,
}

impl RawMutex {
    const fn new() -> Self {
        Self {
            state: AtomicU32::new(MUTEX_UNLOCKED),
        }
    }

    fn lock(&self) {
        if self
            .state
            .compare_exchange(
                MUTEX_UNLOCKED,
                MUTEX_LOCKED,
                Ordering::Acquire,
                Ordering::Relaxed,
            )
            .is_ok()
        {
            return;
        }

        loop {
            if self.state.swap(MUTEX_CONTENDED, Ordering::Acquire) == MUTEX_UNLOCKED {
                return;
            }
            let result = syscall3(
                Syscall::FutexWait,
                &self.state as *const AtomicU32 as usize,
                MUTEX_CONTENDED as usize,
                WAIT_FOREVER,
            );
            if result == usize::MAX {
                thread::sleep(Duration::from_millis(1));
            }
        }
    }

    fn unlock(&self) {
        if self.state.swap(MUTEX_UNLOCKED, Ordering::Release) == MUTEX_CONTENDED {
            let _ = syscall2(
                Syscall::FutexWake,
                &self.state as *const AtomicU32 as usize,
                1,
            );
        }
    }
}

struct RawConditionVariable {
    sequence: AtomicU32,
}

impl RawConditionVariable {
    const fn new() -> Self {
        Self {
            sequence: AtomicU32::new(0),
        }
    }

    fn signal(&self) {
        self.sequence.fetch_add(1, Ordering::Release);
        let _ = syscall2(
            Syscall::FutexWake,
            &self.sequence as *const AtomicU32 as usize,
            1,
        );
    }

    fn wait(&self, mutex: &RawMutex) {
        let sequence = self.sequence.load(Ordering::Acquire);
        mutex.unlock();
        let _ = syscall3(
            Syscall::FutexWait,
            &self.sequence as *const AtomicU32 as usize,
            sequence as usize,
            WAIT_FOREVER,
        );
        mutex.lock();
    }
}

type ThreadEntry = unsafe extern "C" fn(*mut c_void) -> *mut c_void;

fn set_errno(error: c_int) {
    ERRNO.with(|errno| errno.set(error));
}

fn decode_syscall(result: usize) -> Result<usize, c_int> {
    if result <= isize::MAX as usize {
        Ok(result)
    } else {
        let error = -(result as isize) as c_int;
        Err(if error == 1 { EIO } else { error })
    }
}

fn return_int(result: Result<usize, c_int>) -> c_int {
    match result {
        Ok(value) => value as c_int,
        Err(error) => {
            set_errno(error);
            -1
        }
    }
}

fn return_size(result: Result<usize, c_int>) -> isize {
    match result {
        Ok(value) => value as isize,
        Err(error) => {
            set_errno(error);
            -1
        }
    }
}

fn allocation_layout(size: usize, alignment: usize) -> Option<Layout> {
    let alignment = alignment.max(16);
    if !alignment.is_power_of_two() {
        return None;
    }
    size.max(1)
        .checked_add(std::mem::size_of::<AllocationHeader>())?
        .checked_add(alignment - 1)
        .and_then(|total| Layout::from_size_align(total, alignment).ok())
}

unsafe fn allocate_with_alignment(size: usize, alignment: usize) -> *mut c_void {
    let Some(layout) = allocation_layout(size, alignment) else {
        set_errno(ENOMEM);
        return ptr::null_mut();
    };
    // SAFETY: `layout` is non-zero and valid.
    let base = unsafe { alloc(layout) };
    if base.is_null() {
        set_errno(ENOMEM);
        return ptr::null_mut();
    }
    let header_size = std::mem::size_of::<AllocationHeader>();
    let payload_address =
        (base as usize + header_size + layout.align() - 1) & !(layout.align() - 1);
    let header = (payload_address - header_size) as *mut AllocationHeader;
    // SAFETY: The rounded payload and its immediately preceding header both
    // reside within the allocation described by `layout`.
    unsafe {
        header.write(AllocationHeader {
            base,
            layout_size: layout.size(),
            layout_align: layout.align(),
            requested_size: size,
            magic: ALLOCATION_MAGIC,
        });
    }
    payload_address as *mut c_void
}

unsafe fn sockaddr_v4(address: *const SockAddr, length: u32) -> Result<NativeInet4Address, c_int> {
    if address.is_null() || length < std::mem::size_of::<SockAddrIn>() as u32 {
        return Err(EINVAL);
    }
    // SAFETY: The caller supplied a non-null socket address at least as large
    // as `SockAddrIn`; C layout is shared with the Scarlet compatibility header.
    let address = unsafe { &*address.cast::<SockAddrIn>() };
    if address.family as c_int != AF_INET {
        return Err(EAFNOSUPPORT);
    }
    Ok(NativeInet4Address {
        address: address.address.to_ne_bytes(),
        port: u16::from_be(address.port),
    })
}

fn wire_address(address: NativeInet4Address) -> [u8; 8] {
    let mut wire = [0; 8];
    wire[0] = AF_INET as u8;
    wire[2..6].copy_from_slice(&address.address);
    wire[6..8].copy_from_slice(&address.port.to_be_bytes());
    wire
}

unsafe fn write_sockaddr(
    address: *mut SockAddr,
    length: *mut u32,
    wire: &[u8; 8],
) -> Result<(), c_int> {
    if address.is_null() {
        return Ok(());
    }
    if length.is_null() {
        return Err(EINVAL);
    }
    // SAFETY: `length` is required to point to a writable C `socklen_t`.
    let available = unsafe { *length };
    // SAFETY: The same pointer is writable under the caller contract.
    unsafe { *length = std::mem::size_of::<SockAddrIn>() as u32 };
    if available < std::mem::size_of::<SockAddrIn>() as u32 {
        return Err(EMSGSIZE);
    }
    if wire[0] != AF_INET as u8 {
        return Err(EAFNOSUPPORT);
    }
    let value = SockAddrIn {
        family: AF_INET as u16,
        port: u16::from_be_bytes([wire[6], wire[7]]).to_be(),
        address: u32::from_ne_bytes([wire[2], wire[3], wire[4], wire[5]]),
        zero: [0; 8],
    };
    // SAFETY: The caller reported enough writable storage for `SockAddrIn`.
    unsafe { address.cast::<SockAddrIn>().write(value) };
    Ok(())
}

fn query_socket_address(socket: c_int, peer: bool) -> Result<[u8; 8], c_int> {
    let mut address = [0; 8];
    let syscall = if peer {
        Syscall::SocketGetPeerAddress
    } else {
        Syscall::SocketGetLocalAddress
    };
    decode_syscall(syscall2(
        syscall,
        socket as usize,
        address.as_mut_ptr() as usize,
    ))?;
    Ok(address)
}

fn set_socket_control(socket: c_int, command: u32, value: usize) -> Result<usize, c_int> {
    decode_syscall(syscall3(
        Syscall::HandleControl,
        socket as usize,
        command as usize,
        value,
    ))
}

#[unsafe(no_mangle)]
extern "C" fn scarlet_errno_location() -> *mut c_int {
    ERRNO.with(Cell::as_ptr)
}

#[unsafe(no_mangle)]
extern "C" fn scarlet_monotonic_time_ns() -> u64 {
    syscall0(Syscall::MonotonicTime) as u64
}

#[unsafe(no_mangle)]
extern "C" fn scarlet_sleep_us(microseconds: u64) {
    thread::sleep(Duration::from_micros(microseconds));
}

#[unsafe(no_mangle)]
unsafe extern "C" fn scarlet_thread_create(
    name: *const c_char,
    entry: ThreadEntry,
    context: *mut c_void,
) -> usize {
    let context = context as usize;
    let mut builder = thread::Builder::new();
    if !name.is_null() {
        // SAFETY: The C caller provides a NUL-terminated name for this call.
        let name = unsafe { CStr::from_ptr(name) }
            .to_string_lossy()
            .into_owned();
        builder = builder.name(name);
    }
    match builder.spawn(move || {
        // SAFETY: The C core owns the entry point and context until it returns.
        let _ = unsafe { entry(context as *mut c_void) };
    }) {
        Ok(thread) => Box::into_raw(Box::new(thread)) as usize,
        Err(_) => 0,
    }
}

#[unsafe(no_mangle)]
unsafe extern "C" fn scarlet_thread_join(handle: usize) {
    if handle == 0 {
        return;
    }
    // SAFETY: The handle was produced by `scarlet_thread_create` and is consumed once.
    let thread = unsafe { Box::from_raw(handle as *mut thread::JoinHandle<()>) };
    let _ = thread.join();
}

#[unsafe(no_mangle)]
unsafe extern "C" fn scarlet_thread_detach(handle: usize) {
    if handle != 0 {
        // SAFETY: Dropping this unique boxed join handle detaches the thread.
        drop(unsafe { Box::from_raw(handle as *mut thread::JoinHandle<()>) });
    }
}

#[unsafe(no_mangle)]
extern "C" fn scarlet_mutex_create() -> usize {
    Box::into_raw(Box::new(RawMutex::new())) as usize
}

#[unsafe(no_mangle)]
unsafe extern "C" fn scarlet_mutex_destroy(handle: usize) {
    if handle != 0 {
        // SAFETY: The handle is a unique allocation returned by `scarlet_mutex_create`.
        drop(unsafe { Box::from_raw(handle as *mut RawMutex) });
    }
}

#[unsafe(no_mangle)]
unsafe extern "C" fn scarlet_mutex_lock(handle: usize) {
    // SAFETY: Live mutex handles always point to `RawMutex`.
    unsafe { &*(handle as *const RawMutex) }.lock();
}

#[unsafe(no_mangle)]
unsafe extern "C" fn scarlet_mutex_unlock(handle: usize) {
    // SAFETY: Live mutex handles always point to `RawMutex`.
    unsafe { &*(handle as *const RawMutex) }.unlock();
}

#[unsafe(no_mangle)]
extern "C" fn scarlet_cond_create() -> usize {
    Box::into_raw(Box::new(RawConditionVariable::new())) as usize
}

#[unsafe(no_mangle)]
unsafe extern "C" fn scarlet_cond_destroy(handle: usize) {
    if handle != 0 {
        // SAFETY: The handle is a unique allocation returned by `scarlet_cond_create`.
        drop(unsafe { Box::from_raw(handle as *mut RawConditionVariable) });
    }
}

#[unsafe(no_mangle)]
unsafe extern "C" fn scarlet_cond_signal(handle: usize) {
    // SAFETY: Live condition handles always point to `RawConditionVariable`.
    unsafe { &*(handle as *const RawConditionVariable) }.signal();
}

#[unsafe(no_mangle)]
unsafe extern "C" fn scarlet_cond_wait(cond_handle: usize, mutex_handle: usize) {
    // SAFETY: Both handles remain live for the duration of the wait.
    let cond = unsafe { &*(cond_handle as *const RawConditionVariable) };
    // SAFETY: Both handles remain live for the duration of the wait.
    let mutex = unsafe { &*(mutex_handle as *const RawMutex) };
    cond.wait(mutex);
}

#[unsafe(no_mangle)]
unsafe extern "C" fn malloc(size: usize) -> *mut c_void {
    // SAFETY: `malloc` requires alignment suitable for every scalar C type.
    unsafe { allocate_with_alignment(size, 16) }
}

#[unsafe(no_mangle)]
unsafe extern "C" fn aligned_alloc(alignment: usize, size: usize) -> *mut c_void {
    if alignment == 0 || !alignment.is_power_of_two() || size % alignment != 0 {
        set_errno(EINVAL);
        return ptr::null_mut();
    }
    // SAFETY: Alignment and size were validated above.
    unsafe { allocate_with_alignment(size, alignment) }
}

#[unsafe(no_mangle)]
unsafe extern "C" fn calloc(count: usize, size: usize) -> *mut c_void {
    let Some(total) = count.checked_mul(size) else {
        set_errno(ENOMEM);
        return ptr::null_mut();
    };
    // SAFETY: Delegating allocation to the matching C ABI allocator.
    let allocation = unsafe { malloc(total) };
    if !allocation.is_null() {
        // SAFETY: `allocation` has at least `total` writable bytes.
        unsafe { allocation.cast::<u8>().write_bytes(0, total) };
    }
    allocation
}

#[unsafe(no_mangle)]
unsafe extern "C" fn free(pointer: *mut c_void) {
    if pointer.is_null() {
        return;
    }
    // SAFETY: C callers may only free pointers returned by this allocator.
    let header = unsafe {
        pointer
            .cast::<u8>()
            .sub(std::mem::size_of::<AllocationHeader>())
            .cast::<AllocationHeader>()
    };
    // SAFETY: A live allocation always contains its header at `base`.
    let header = unsafe { &*header };
    if header.magic != ALLOCATION_MAGIC {
        return;
    }
    let Ok(layout) = Layout::from_size_align(header.layout_size, header.layout_align) else {
        return;
    };
    // SAFETY: `base` and `layout` describe the original live allocation.
    unsafe { dealloc(header.base, layout) };
}

#[unsafe(no_mangle)]
unsafe extern "C" fn realloc_c(pointer: *mut c_void, size: usize) -> *mut c_void {
    if pointer.is_null() {
        // SAFETY: This is the C `realloc(NULL, size)` case.
        return unsafe { malloc(size) };
    }
    if size == 0 {
        // SAFETY: This is the C `realloc(pointer, 0)` case.
        unsafe { free(pointer) };
        return ptr::null_mut();
    }
    // SAFETY: C callers may only reallocate pointers returned by this allocator.
    let header = unsafe {
        pointer
            .cast::<u8>()
            .sub(std::mem::size_of::<AllocationHeader>())
            .cast::<AllocationHeader>()
    };
    // SAFETY: A live allocation always contains its header at `base`.
    let header = unsafe { &*header };
    if header.magic != ALLOCATION_MAGIC {
        set_errno(EINVAL);
        return ptr::null_mut();
    }
    let old_size = header.requested_size;
    let old_alignment = header.layout_align;
    // SAFETY: Preserve the original allocation's alignment.
    let replacement = unsafe { allocate_with_alignment(size, old_alignment) };
    if replacement.is_null() {
        return ptr::null_mut();
    }
    // SAFETY: Both allocations are live and non-overlapping, and the copied
    // length is bounded by both payload sizes.
    unsafe {
        ptr::copy_nonoverlapping(
            pointer.cast::<u8>(),
            replacement.cast::<u8>(),
            old_size.min(size),
        );
        free(pointer);
    }
    replacement
}

// Export the exact libc spelling without shadowing Rust's imported `realloc`.
#[unsafe(export_name = "realloc")]
unsafe extern "C" fn export_realloc(pointer: *mut c_void, size: usize) -> *mut c_void {
    // SAFETY: The ABI contract is forwarded unchanged.
    unsafe { realloc_c(pointer, size) }
}

#[unsafe(no_mangle)]
extern "C" fn scarlet_write_bytes(bytes: *const c_char, length: usize) {
    if !bytes.is_null() && length != 0 {
        let _ = syscall3(Syscall::StreamWrite, 1, bytes as usize, length);
    }
}

#[unsafe(no_mangle)]
extern "C" fn scarlet_abort() -> ! {
    std::process::abort()
}

static FALLBACK_RANDOM: AtomicU64 = AtomicU64::new(0x8f6d_4a21_d3c7_b509);

#[unsafe(no_mangle)]
extern "C" fn scarlet_random_u32() -> u32 {
    let mut output = 0u32;
    let result = syscall3(
        Syscall::GetRandom,
        &mut output as *mut u32 as usize,
        std::mem::size_of::<u32>(),
        0,
    );
    if result == std::mem::size_of::<u32>() {
        return output;
    }
    let mut value = FALLBACK_RANDOM.load(Ordering::Relaxed);
    loop {
        let next = value ^ (value << 13) ^ (value >> 7) ^ (value << 17);
        match FALLBACK_RANDOM.compare_exchange_weak(
            value,
            next,
            Ordering::Relaxed,
            Ordering::Relaxed,
        ) {
            Ok(_) => return next as u32,
            Err(current) => value = current,
        }
    }
}

#[unsafe(no_mangle)]
unsafe extern "C" fn mbedtls_hardware_poll(
    _data: *mut c_void,
    output: *mut u8,
    length: usize,
    output_length: *mut usize,
) -> c_int {
    if output.is_null() || output_length.is_null() {
        return -1;
    }
    let result = syscall3(
        Syscall::GetRandom,
        output as usize,
        length,
        GET_RANDOM_FLAG_REQUIRE_ENTROPY,
    );
    if result != length {
        // SAFETY: The caller supplied a writable output length pointer.
        unsafe { *output_length = 0 };
        return -1;
    }
    // SAFETY: The caller supplied a writable output length pointer.
    unsafe { *output_length = length };
    0
}

#[unsafe(no_mangle)]
extern "C" fn socket(domain: c_int, socket_type: c_int, protocol: c_int) -> c_int {
    if domain != AF_INET {
        set_errno(EAFNOSUPPORT);
        return -1;
    }
    return_int(decode_syscall(syscall3(
        Syscall::SocketCreate,
        domain as usize,
        socket_type as usize,
        protocol as usize,
    )))
}

#[unsafe(no_mangle)]
unsafe extern "C" fn bind(socket: c_int, address: *const SockAddr, length: u32) -> c_int {
    // SAFETY: The C socket ABI owns the address pointer and length validation.
    let address = match unsafe { sockaddr_v4(address, length) } {
        Ok(address) => address,
        Err(error) => {
            set_errno(error);
            return -1;
        }
    };
    return_int(decode_syscall(syscall3(
        Syscall::SocketBind,
        socket as usize,
        &address as *const NativeInet4Address as usize,
        std::mem::size_of::<NativeInet4Address>(),
    )))
}

#[unsafe(no_mangle)]
unsafe extern "C" fn connect(socket: c_int, address: *const SockAddr, length: u32) -> c_int {
    // SAFETY: The C socket ABI owns the address pointer and length validation.
    let address = match unsafe { sockaddr_v4(address, length) } {
        Ok(address) => address,
        Err(error) => {
            set_errno(error);
            return -1;
        }
    };
    return_int(decode_syscall(syscall3(
        Syscall::SocketConnect,
        socket as usize,
        &address as *const NativeInet4Address as usize,
        std::mem::size_of::<NativeInet4Address>(),
    )))
}

#[unsafe(no_mangle)]
extern "C" fn listen(socket: c_int, backlog: c_int) -> c_int {
    return_int(decode_syscall(syscall2(
        Syscall::SocketListen,
        socket as usize,
        backlog.max(0) as usize,
    )))
}

#[unsafe(no_mangle)]
unsafe extern "C" fn accept(socket: c_int, address: *mut SockAddr, length: *mut u32) -> c_int {
    let accepted = match decode_syscall(syscall1(Syscall::SocketAccept, socket as usize)) {
        Ok(accepted) => accepted,
        Err(error) => {
            set_errno(error);
            return -1;
        }
    };
    if !address.is_null() {
        let result = query_socket_address(accepted as c_int, true)
            .and_then(|wire| unsafe { write_sockaddr(address, length, &wire) });
        if let Err(error) = result {
            let _ = syscall1(Syscall::HandleClose, accepted);
            set_errno(error);
            return -1;
        }
    }
    accepted as c_int
}

#[unsafe(no_mangle)]
extern "C" fn shutdown(socket: c_int, how: c_int) -> c_int {
    return_int(decode_syscall(syscall2(
        Syscall::SocketShutdown,
        socket as usize,
        how as usize,
    )))
}

#[unsafe(no_mangle)]
extern "C" fn close(socket: c_int) -> c_int {
    return_int(decode_syscall(syscall1(
        Syscall::HandleClose,
        socket as usize,
    )))
}

#[unsafe(no_mangle)]
unsafe extern "C" fn getsockname(socket: c_int, address: *mut SockAddr, length: *mut u32) -> c_int {
    match query_socket_address(socket, false)
        .and_then(|wire| unsafe { write_sockaddr(address, length, &wire) })
    {
        Ok(()) => 0,
        Err(error) => {
            set_errno(error);
            -1
        }
    }
}

#[unsafe(no_mangle)]
unsafe extern "C" fn getpeername(socket: c_int, address: *mut SockAddr, length: *mut u32) -> c_int {
    match query_socket_address(socket, true)
        .and_then(|wire| unsafe { write_sockaddr(address, length, &wire) })
    {
        Ok(()) => 0,
        Err(error) => {
            set_errno(error);
            -1
        }
    }
}

#[unsafe(no_mangle)]
unsafe extern "C" fn send(
    socket: c_int,
    buffer: *const c_void,
    length: usize,
    flags: c_int,
) -> isize {
    if buffer.is_null() && length != 0 || flags != 0 {
        set_errno(EINVAL);
        return -1;
    }
    return_size(decode_syscall(syscall3(
        Syscall::StreamWrite,
        socket as usize,
        buffer as usize,
        length,
    )))
}

#[unsafe(no_mangle)]
unsafe extern "C" fn recv(
    socket: c_int,
    buffer: *mut c_void,
    length: usize,
    flags: c_int,
) -> isize {
    if buffer.is_null() && length != 0 || flags != 0 {
        set_errno(if flags != 0 { EOPNOTSUPP } else { EINVAL });
        return -1;
    }
    return_size(decode_syscall(syscall3(
        Syscall::StreamRead,
        socket as usize,
        buffer as usize,
        length,
    )))
}

#[unsafe(no_mangle)]
unsafe extern "C" fn sendto(
    socket: c_int,
    buffer: *const c_void,
    length: usize,
    flags: c_int,
    address: *const SockAddr,
    address_length: u32,
) -> isize {
    if buffer.is_null() && length != 0 || flags != 0 {
        set_errno(EINVAL);
        return -1;
    }
    if address.is_null() {
        // SAFETY: Pointer and length have already been validated for stream write.
        return unsafe { send(socket, buffer, length, 0) };
    }
    // SAFETY: The C socket ABI owns the address pointer and length validation.
    let address = match unsafe { sockaddr_v4(address, address_length) } {
        Ok(address) => wire_address(address),
        Err(error) => {
            set_errno(error);
            return -1;
        }
    };
    return_size(decode_syscall(syscall4(
        Syscall::SocketSendTo,
        socket as usize,
        buffer as usize,
        length,
        address.as_ptr() as usize,
    )))
}

#[unsafe(no_mangle)]
unsafe extern "C" fn recvfrom(
    socket: c_int,
    buffer: *mut c_void,
    length: usize,
    flags: c_int,
    address: *mut SockAddr,
    address_length: *mut u32,
) -> isize {
    if buffer.is_null() && length != 0 || flags != 0 {
        set_errno(if flags != 0 { EOPNOTSUPP } else { EINVAL });
        return -1;
    }
    let mut wire = [0; 8];
    let result = decode_syscall(syscall4(
        Syscall::SocketRecvFrom,
        socket as usize,
        buffer as usize,
        length,
        if address.is_null() {
            0
        } else {
            wire.as_mut_ptr() as usize
        },
    ));
    let received = match result {
        Ok(received) => received,
        Err(error) => {
            set_errno(error);
            return -1;
        }
    };
    if let Err(error) = unsafe { write_sockaddr(address, address_length, &wire) } {
        set_errno(error);
        return -1;
    }
    received as isize
}

#[unsafe(no_mangle)]
unsafe extern "C" fn setsockopt(
    socket: c_int,
    level: c_int,
    option: c_int,
    value: *const c_void,
    length: u32,
) -> c_int {
    if value.is_null() {
        set_errno(EINVAL);
        return -1;
    }
    let result = if level == SOL_SOCKET && (option == SO_RCVTIMEO || option == SO_SNDTIMEO) {
        if length < std::mem::size_of::<TimeVal>() as u32 {
            Err(EINVAL)
        } else {
            // SAFETY: The option buffer is at least one `TimeVal` long.
            let timeout = unsafe { &*value.cast::<TimeVal>() };
            let millis = timeout
                .seconds
                .saturating_mul(1000)
                .saturating_add((timeout.microseconds.saturating_add(999)) / 1000)
                .max(0) as usize;
            let command = if option == SO_RCVTIMEO {
                SCTL_SOCKET_SET_READ_TIMEOUT_MS
            } else {
                SCTL_SOCKET_SET_WRITE_TIMEOUT_MS
            };
            set_socket_control(socket, command, millis)
        }
    } else if level == SOL_SOCKET && option == SO_NONBLOCK {
        if length < std::mem::size_of::<c_int>() as u32 {
            Err(EINVAL)
        } else {
            // SAFETY: The option buffer is at least one C integer long.
            let enabled = unsafe { *value.cast::<c_int>() } != 0;
            set_socket_control(socket, SCTL_SOCKET_SET_NONBLOCK, usize::from(enabled))
        }
    } else if (level == SOL_SOCKET
        && matches!(option, SO_REUSEADDR | SO_BROADCAST | SO_SNDBUF | SO_RCVBUF))
        || (level == IPPROTO_IP && option == IP_TTL)
    {
        Ok(0)
    } else {
        Ok(0)
    };
    match result {
        Ok(_) => 0,
        Err(error) => {
            set_errno(error);
            -1
        }
    }
}

#[unsafe(no_mangle)]
unsafe extern "C" fn getsockopt(
    socket: c_int,
    level: c_int,
    option: c_int,
    value: *mut c_void,
    length: *mut u32,
) -> c_int {
    if value.is_null() || length.is_null() {
        set_errno(EINVAL);
        return -1;
    }
    // SAFETY: `length` is a valid writable socklen pointer by ABI contract.
    if unsafe { *length } < std::mem::size_of::<c_int>() as u32 {
        set_errno(EINVAL);
        return -1;
    }
    let result = if level == SOL_SOCKET && option == SO_ERROR {
        set_socket_control(socket, SCTL_SOCKET_TAKE_ERROR, 0).map(|error| error as c_int)
    } else if level == IPPROTO_IP && option == IP_TTL {
        Ok(64)
    } else if level == SOL_SOCKET && (option == SO_SNDBUF || option == SO_RCVBUF) {
        Ok(256 * 1024)
    } else {
        Err(EOPNOTSUPP)
    };
    match result {
        Ok(result) => {
            // SAFETY: The caller provided room for a C integer.
            unsafe {
                *value.cast::<c_int>() = result;
                *length = std::mem::size_of::<c_int>() as u32;
            }
            0
        }
        Err(error) => {
            set_errno(error);
            -1
        }
    }
}

#[unsafe(no_mangle)]
extern "C" fn scarlet_socket_set_nonblocking(socket: c_int, enabled: c_int) -> c_int {
    match set_socket_control(socket, SCTL_SOCKET_SET_NONBLOCK, usize::from(enabled != 0)) {
        Ok(_) => 0,
        Err(error) => {
            set_errno(error);
            -1
        }
    }
}

#[unsafe(no_mangle)]
unsafe extern "C" fn poll(descriptors: *mut PollFd, count: usize, timeout_ms: c_int) -> c_int {
    if descriptors.is_null() && count != 0 {
        set_errno(EINVAL);
        return -1;
    }
    let timeout_ns = if timeout_ms < 0 {
        -1
    } else {
        i64::from(timeout_ms).saturating_mul(1_000_000)
    };
    let options = PollOptions {
        timeout_ns,
        min_timeout_ns: 0,
    };
    return_int(decode_syscall(syscall3(
        Syscall::Poll,
        descriptors as usize,
        count,
        &options as *const PollOptions as usize,
    )))
}

#[unsafe(no_mangle)]
unsafe extern "C" fn inet_pton(
    family: c_int,
    source: *const c_char,
    destination: *mut c_void,
) -> c_int {
    if family != AF_INET {
        set_errno(EAFNOSUPPORT);
        return -1;
    }
    if source.is_null() || destination.is_null() {
        set_errno(EINVAL);
        return -1;
    }
    // SAFETY: `source` is a NUL-terminated C string by ABI contract.
    let Ok(source) = unsafe { CStr::from_ptr(source) }.to_str() else {
        return 0;
    };
    let Ok(address) = Ipv4Addr::from_str(source) else {
        return 0;
    };
    // SAFETY: IPv4 `inet_pton` callers provide four writable bytes.
    unsafe { ptr::copy_nonoverlapping(address.octets().as_ptr(), destination.cast::<u8>(), 4) };
    1
}

#[unsafe(no_mangle)]
unsafe extern "C" fn inet_ntop(
    family: c_int,
    source: *const c_void,
    destination: *mut c_char,
    length: u32,
) -> *const c_char {
    if family != AF_INET || source.is_null() || destination.is_null() {
        set_errno(if family == AF_INET {
            EINVAL
        } else {
            EAFNOSUPPORT
        });
        return ptr::null();
    }
    // SAFETY: IPv4 `inet_ntop` callers provide four readable bytes.
    let octets = unsafe { std::slice::from_raw_parts(source.cast::<u8>(), 4) };
    let text = Ipv4Addr::new(octets[0], octets[1], octets[2], octets[3]).to_string();
    if text.len() + 1 > length as usize {
        set_errno(EMSGSIZE);
        return ptr::null();
    }
    // SAFETY: The caller provided the capacity reported by `length`.
    unsafe {
        ptr::copy_nonoverlapping(text.as_ptr(), destination.cast::<u8>(), text.len());
        *destination.add(text.len()) = 0;
    }
    destination
}

#[unsafe(no_mangle)]
unsafe extern "C" fn getaddrinfo(
    node: *const c_char,
    service: *const c_char,
    hints: *const AddrInfo,
    result: *mut *mut AddrInfo,
) -> c_int {
    if node.is_null() || result.is_null() {
        return EAI_NONAME;
    }
    // SAFETY: `node` is a NUL-terminated string by ABI contract.
    let Ok(node) = unsafe { CStr::from_ptr(node) }.to_str() else {
        return EAI_NONAME;
    };
    let port = if service.is_null() {
        0
    } else {
        // SAFETY: Non-null service strings are NUL terminated by ABI contract.
        let Ok(service) = unsafe { CStr::from_ptr(service) }.to_str() else {
            return EAI_NONAME;
        };
        let Ok(port) = service.parse::<u16>() else {
            return EAI_NONAME;
        };
        port
    };
    // SAFETY: A non-null hints pointer points to a complete `AddrInfo`.
    let family = if hints.is_null() {
        AF_UNSPEC
    } else {
        unsafe { (*hints).family }
    };
    if family != AF_UNSPEC && family != AF_INET {
        return EAI_NONAME;
    }
    let addresses = match (node, port).to_socket_addrs() {
        Ok(addresses) => addresses,
        Err(_) => return EAI_FAIL,
    };

    let mut head: *mut AddrInfo = ptr::null_mut();
    let mut tail: *mut AddrInfo = ptr::null_mut();
    for address in addresses {
        let SocketAddr::V4(address) = address else {
            continue;
        };
        let mut node = Box::new(AddrInfoNode {
            info: AddrInfo {
                flags: 0,
                family: AF_INET,
                socket_type: if hints.is_null() {
                    0
                } else {
                    // SAFETY: Validated non-null hints pointer above.
                    unsafe { (*hints).socket_type }
                },
                protocol: if hints.is_null() {
                    0
                } else {
                    // SAFETY: Validated non-null hints pointer above.
                    unsafe { (*hints).protocol }
                },
                address_length: std::mem::size_of::<SockAddrIn>() as u32,
                canonical_name: ptr::null_mut(),
                address: ptr::null_mut(),
                next: ptr::null_mut(),
            },
            address: SockAddrIn {
                family: AF_INET as u16,
                port: address.port().to_be(),
                address: u32::from_ne_bytes(address.ip().octets()),
                zero: [0; 8],
            },
        });
        node.info.address = (&mut node.address as *mut SockAddrIn).cast();
        let current = Box::into_raw(node).cast::<AddrInfo>();
        if head.is_null() {
            head = current;
        } else {
            // SAFETY: `tail` is the final live node in the list.
            unsafe { (*tail).next = current };
        }
        tail = current;
    }
    if head.is_null() {
        return EAI_NONAME;
    }
    // SAFETY: `result` is a writable pointer supplied by the C caller.
    unsafe { *result = head };
    0
}

#[unsafe(no_mangle)]
unsafe extern "C" fn freeaddrinfo(mut result: *mut AddrInfo) {
    while !result.is_null() {
        // SAFETY: Each node was allocated by `getaddrinfo` and is consumed once.
        let node = unsafe { Box::from_raw(result.cast::<AddrInfoNode>()) };
        result = node.info.next;
        drop(node);
    }
}

// Make unsupported protocol failures explicit for callers that probe directly.
#[allow(dead_code)]
fn unsupported_protocol() -> c_int {
    set_errno(EPROTONOSUPPORT);
    -1
}
