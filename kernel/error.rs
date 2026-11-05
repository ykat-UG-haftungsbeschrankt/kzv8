/*
 * Copyright (c) 2026 ykat UG (haftungsbeschraenkt) - All Rights Reserved
 *
 * Permission for non-commercial use is hereby granted,
 * free of charge, without warranty of any kind.
 */

#[repr(i32)]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[allow(dead_code)]
pub(crate) enum Error {
    /// Minimum value of zrtos_error_t.
    Min = i32::MIN,
    /// Successful program execution status.
    Success = 0,
    /// Unsuccessful program execution status.
    Error = -1,
    /// No such file or directory.
    NoEnt = -2,
    /// No such process.
    Srch = -3,
    /// Interrupted system call.
    Intr = -4,
    /// I/O error.
    Io = -5,
    /// No such device or address.
    Nxio = -6,
    /// Argument list too long.
    TooBig = -7,
    /// Exec format error.
    NoExec = -8,
    /// Bad file number.
    Badf = -9,
    /// No child processes.
    Child = -10,
    /// Try again.
    Again = -11,
    /// Out of memory.
    NoMem = -12,
    /// Permission denied.
    Acces = -13,
    /// Bad address.
    Fault = -14,
    /// Block device required.
    NotBlk = -15,
    /// Device or resource busy.
    Busy = -16,
    /// File exists.
    Exist = -17,
    /// Cross-device link.
    Xdev = -18,
    /// No such device.
    NoDev = -19,
    /// Not a directory.
    NotDir = -20,
    /// Is a directory.
    IsDir = -21,
    /// Invalid argument.
    Inval = -22,
    /// File table overflow.
    Nfile = -23,
    /// Too many open files.
    Mfile = -24,
    /// Not a typewriter.
    NotTty = -25,
    /// Text file busy.
    TxtBsy = -26,
    /// File too large.
    Fbig = -27,
    /// No space left on device.
    NoSpc = -28,
    /// Illegal seek.
    Spipe = -29,
    /// Read-only file system.
    Rofs = -30,
    /// Too many links.
    Mlink = -31,
    /// Broken pipe.
    Pipe = -32,
    /// Math argument out of domain of function.
    Dom = -33,
    /// Math result not representable.
    Range = -34,
    /// Resource deadlock would occur.
    Deadlk = -35,
    /// File name too long.
    NameTooLong = -36,
    /// No record locks available.
    Nolck = -37,
    /// Function not implemented.
    NoSys = -38,
    /// Directory not empty.
    NotEmpty = -39,
    /// Too many symbolic links encountered.
    Loop = -40,
    /// Operation would block.
    WouldBlock = -41,
    /// No message of desired type.
    NoMsg = -42,
    /// Identifier removed.
    Idrm = -43,
    /// Channel number out of range.
    Chrng = -44,
    /// Level 2 not synchronized.
    L2Nsync = -45,
    /// Level 3 halted.
    L3Hlt = -46,
    /// Level 3 reset.
    L3Rst = -47,
    /// Link number out of range.
    LnRng = -48,
    /// Protocol driver not attached.
    Unatch = -49,
    /// No CSI structure available.
    NoCsi = -50,
    /// Level 2 halted.
    L2Hlt = -51,
    /// Invalid exchange.
    Bade = -52,
    /// Invalid request descriptor.
    Badr = -53,
    /// Exchange full.
    Xfull = -54,
    /// No anode.
    Noano = -55,
    /// Invalid request code.
    Badrqc = -56,
    /// Invalid slot.
    Badslt = -57,
    /// File locking deadlock error.
    Deadlock = -58,
    /// Bad font file format.
    Bfont = -59,
    /// Device not a stream.
    Nostr = -60,
    /// No data available.
    NoData = -61,
    /// Timer expired.
    Time = -62,
    /// Out of streams resources.
    Nosr = -63,
    /// Machine is not on the network.
    Nonet = -64,
    /// Package not installed.
    Nopkg = -65,
    /// Object is remote.
    Remote = -66,
    /// Link has been severed.
    NoLink = -67,
    /// Advertise error.
    Adv = -68,
    /// Srmount error.
    Srmnt = -69,
    /// Communication error on send.
    Comm = -70,
    /// Protocol error.
    Proto = -71,
    /// Multihop attempted.
    Multihop = -72,
    /// RFS specific error.
    DotDot = -73,
    /// Not a data message.
    BadMsg = -74,
    /// Value too large for defined data type.
    Overflow = -75,
    /// Name not unique on network.
    NotUniq = -76,
    /// File descriptor in bad state.
    BadFd = -77,
    /// Remote address changed.
    Remchg = -78,
    /// Cannot access a needed shared library.
    Libacc = -79,
    /// Accessing a corrupted shared library.
    Libbad = -80,
    /// .lib section in a.out corrupted.
    Libscn = -81,
    /// Attempting to link in too many shared libraries.
    Libmax = -82,
    /// Cannot exec a shared library directly.
    Libexec = -83,
    /// Illegal byte sequence.
    Ilseq = -84,
    /// Interrupted system call should be restarted.
    Restart = -85,
    /// Streams pipe error.
    Strpipe = -86,
    /// Too many users.
    Users = -87,
    /// Socket operation on non-socket.
    NotSock = -88,
    /// Destination address required.
    DestAddrReq = -89,
    /// Message too long.
    MsgSize = -90,
    /// Protocol wrong type for socket.
    Prototype = -91,
    /// Protocol not available.
    NoProtoOpt = -92,
    /// Protocol not supported.
    ProtoNoSupport = -93,
    /// Socket type not supported.
    SockTypeNoSupport = -94,
    /// Operation not supported on transport endpoint.
    OpNotSupp = -95,
    /// Protocol family not supported.
    PfNoSupport = -96,
    /// Address family not supported by protocol.
    AfNoSupport = -97,
    /// Address already in use.
    AddrInUse = -98,
    /// Cannot assign requested address.
    AddrNotAvail = -99,
    /// Network is down.
    NetDown = -100,
    /// Network is unreachable.
    NetUnreach = -101,
    /// Network dropped connection because of reset.
    NetReset = -102,
    /// Software caused connection abort.
    ConnAborted = -103,
    /// Connection reset by peer.
    ConnReset = -104,
    /// No buffer space available.
    NoBufs = -105,
    /// Transport endpoint is already connected.
    IsConn = -106,
    /// Transport endpoint is not connected.
    NotConn = -107,
    /// Cannot send after transport endpoint shutdown.
    Shutdown = -108,
    /// Too many references: cannot splice.
    TooManyRefs = -109,
    /// Connection timed out.
    TimedOut = -110,
    /// Connection refused.
    ConnRefused = -111,
    /// Host is down.
    HostDown = -112,
    /// No route to host.
    HostUnreach = -113,
    /// Operation already in progress.
    Already = -114,
    /// Operation now in progress.
    InProgress = -115,
    /// Stale NFS file handle.
    Stale = -116,
    /// Structure needs cleaning.
    Uclean = -117,
    /// Not a XENIX named type file.
    NotNam = -118,
    /// No XENIX semaphores available.
    Navail = -119,
    /// Is a named type file.
    IsNam = -120,
    /// Remote I/O error.
    RemoteIo = -121,
    /// Maximum value of zrtos_error_t.
    Max = i32::MAX,
}
