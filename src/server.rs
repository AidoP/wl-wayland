use std::{fmt, fs::File, any::Any, rc::Rc, cell::RefCell};

use wl::{server::prelude::*, SystemError};
pub use prelude::Global;

#[cfg(feature = "unsealed_shm")]
mod signal;
pub mod prelude {
    /// Derive macro to simplify the implementation of Global interfaces
    pub use global_derive_macro::ServerGlobal as Global;
    pub use super::{
        error,
        drop_handler,
        wayland,
        OnBind,
        Format,

        WlDisplay,
        WlRegistry,
        WlCallback,

        WlShm,
        WlShmPool,
        WlBuffer,

        DisplayError,
        DispatchErrorHandler
    };
}

#[protocol("protocol/wayland.toml")]
pub mod wayland {
    type WlDisplay = super::WlDisplay;
    type WlRegistry = super::WlRegistry;
    type WlCallback = super::WlCallback;

    type WlShm = super::WlShm;
    type WlShmPool = super::WlShmPool;
    type WlBuffer = super::WlBuffer;
}
/// Define error types that leverage the `wl_display.error` event.
/// 
/// ```rust
/// use wl_wayland::server::error;
/// use wl_wayland::WlDisplayError
/// error! {
///     ExampleDisplayError {
///         Implementation = wayland:WlDisplayError::IMPLEMENTATION => implementation {
///             "file descriptor is invalid, {reason}",
///             reason: String
///         },
///         NoMemory = WlDisplayError::NO_MEMORY => out_of_memory {
///             "the system is out of memory, {reason}",
///             reason: String
///         },
///         InvalidObject = WlDisplayError::INVALID_OBJECT => unknown_object {
///             "no object found with the id {id}",
///             id: u32
///         },
///         InvalidMethod = WlDisplayError::INVALID_METHOD => method {
///             "malformed request could not be processed, {reason}",
///             reason: String
///         }
///     }
/// }
/// ```
#[macro_export]
macro_rules! error {
    ($name:ident { $($err_name:ident = $err_val:expr => $constructor:ident {
        $format:expr
        $(,$field:ident: $field_ty:ty)*
    }),* }) => {
        pub enum $name {
            $(
                $err_name {
                    object: u32,
                    $($field: $field_ty),*
                }
            ),*
        }
        impl $name {
            pub fn code(&self) -> u32 {
                match self {
                    $(Self::$err_name {..} => $err_val),*
                }
            }
            pub fn object(&self) -> u32 {
                match self {
                    $(Self::$err_name { object, ..} => *object),*
                }
            }
            $(
                pub fn $constructor(object: &dyn Object $(, $field: $field_ty)*) -> Error {
                    Self::$err_name { object: object.object() $(, $field)* }.into()
                } 
            )*
        }
        impl ErrorHandler for $name {
            fn handle(&mut self, client: &mut Client) -> Result<()> {
                let mut display = WlDisplay::get(client)?;
                {
                    use wayland::WlDisplay;
                    display.error(client, &self.object(), self.code(), &format!("{}", self))?;
                    Err(wl::SystemError::Other(format!("{}", self).into()).into())
                }
            }
        }
        impl fmt::Display for $name {
            fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                match self {
                    $(Self::$err_name { $($field,)* ..} => write!(f, $format, $($field = $field),*)),*
                }
            }
        }
        impl Into<Error> for $name {
            fn into(self) -> Error {
                Error::Protocol(Box::new(self))
            }
        }
    };
}
pub use error;
error! {
    DisplayError {
        Implementation = wayland::WlDisplayError::IMPLEMENTATION => implementation {
            "file descriptor is invalid, {reason}",
            reason: String
        },
        NoMemory = wayland::WlDisplayError::NO_MEMORY => out_of_memory {
            "the system is out of memory, {reason}",
            reason: String
        },
        InvalidObject = wayland::WlDisplayError::INVALID_OBJECT => unknown_object {
            "no object found with the id {id}",
            id: u32
        },
        InvalidMethod = wayland::WlDisplayError::INVALID_METHOD => method {
            "malformed request could not be processed, {reason}",
            reason: String
        }
    }
}

/// Default error handler for dispatch errors
#[derive(Default, Copy, Clone)]
pub struct DispatchErrorHandler;
impl wl::server::DispatchErrorHandler for DispatchErrorHandler {
    fn handle(&mut self, client: &mut Client, error: wl::DispatchError) -> Result<()> {
        let mut display = WlDisplay::get(client)?;
        {
            use wayland::{WlDisplay, WlDisplayError};
            match error {
                wl::DispatchError::ObjectNull
                    => display.error(client, &0, WlDisplayError::INVALID_OBJECT, "Attempted to access the null object (id 0)"),
                wl::DispatchError::ObjectExists(object)
                    => display.error(client, &object, WlDisplayError::INVALID_METHOD, "Cannot add the object as one with that id already exists"),
                wl::DispatchError::ObjectNotFound(object)
                    => display.error(client, &object, WlDisplayError::INVALID_OBJECT, "The specified object does not exist"),
                wl::DispatchError::NoVariant { name, variant }
                    => display.error(client, &Client::DISPLAY, WlDisplayError::INVALID_METHOD, &format!("Enum {:?} does not contain value {}", name, variant)),
                wl::DispatchError::InvalidRequest { opcode, object, interface }
                    => display.error(client, &object, WlDisplayError::INVALID_METHOD, &format!("Interface {:?} has no request with opcode {}", interface, opcode)),
                wl::DispatchError::InvalidEvent { opcode, object, interface }
                    => display.error(client, &object, WlDisplayError::INVALID_METHOD, &format!("Interface {:?} has no event with opcode {}", interface, opcode)),
                wl::DispatchError::UnexpectedObjectType { object, expected_interface, had_interface }
                    => display.error(client, &object, WlDisplayError::INVALID_METHOD, &format!("Expected an object implementing {:?}, but got an object implementing {:?}", expected_interface, had_interface)),
                wl::DispatchError::ExpectedArgument { data_type }
                    => display.error(client, &Client::DISPLAY, WlDisplayError::INVALID_METHOD, &format!("Method arguments corrupt, expected {:?}", data_type)),
                wl::DispatchError::Utf8Error(error)
                    => display.error(client, &Client::DISPLAY, WlDisplayError::INVALID_METHOD, &format!("Only UTF-8 strings are supported, {}", error))
            }
        }
    }
}
/// Default handler for dropped objects, notifying the client of object deletion through the display object
pub fn drop_handler(client: &mut Client, object: Lease<dyn std::any::Any>) -> Result<()> {
    let mut display = WlDisplay::get(client)?;
    {
        use wayland::WlDisplay;
        display.delete_id(client, object.object())
    }
}

#[derive(Clone)]
struct RegisteredGlobal {
    interface: &'static str,
    version: u32,
    global: Box<dyn Global>
}
/// Satisfy the requirement for dyn Global to be Clone.
/// An implementation detail, automatically added for all existing Clone types.
pub trait GlobalClone {
    fn dyn_clone(&self) -> Box<dyn Global>;
}
impl<T: 'static + Global + Clone> GlobalClone for T {
    fn dyn_clone(&self) -> Box<dyn Global> {
        Box::new(self.clone())
    }
}
/// Implemented by types that can be registered as a global. The display object will not be available as it is already leased.
/// 
/// ```rust
/// #[derive(Clone)]
/// struct WlShm;
/// impl WlShm {
///     fn init(lease: Lease<dyn Any>, client: &mut Client, display: &mut Lease<WlDisplay>, registry: &mut Lease<WlRegistry>) -> Result<()> {
///         Ok(())
///     }
/// }
/// impl Global for WlShm {
///     fn construct(&self, client: &mut Client, registry: &mut Lease<WlRegistry>) -> Result<Resident<dyn Any>> {
///         Ok(Client::reserve(Self))
///     }
///     fn init_fn(&self) -> fn(Lease<dyn Any>, &mut Client) -> Result<()> {
///         WlShm::init
///     }
/// }
/// ```
pub trait Global: GlobalClone + Send {
    /// Create a new instance of the object. The display object will not be available as it is already leased.
    fn construct(&self) -> Resident<dyn Any>;
    /// Get a pointer to the initialisation function to be executed on the lease of the constructed object
    fn on_bind_fn(&self) -> fn(Lease<dyn Any>, &mut Client, &mut Lease<WlDisplay>, &mut Lease<WlRegistry>) -> Result<()>;
}
impl Clone for Box<dyn Global> {
    fn clone(&self) -> Self {
        self.dyn_clone()
    }
}
/// Callback for initialisation of a global once a bind has occured
pub trait OnBind {
    #[allow(unused_variables)]
    fn bind(lease: Lease<dyn Any>, client: &mut Client, display: &mut Lease<WlDisplay>, registry: &mut Lease<WlRegistry>) -> Result<()> {
        Ok(())
    }
}

/// The entry point to the Wayland protocol
/// 
/// Bootstraps the protocol by providing a way to get the registry and attached global objects.
/// Also provides synchronisation and error reporting primitives.
#[derive(Clone)]
pub struct WlDisplay {
    serial: u32,
    globals: Vec<Option<RegisteredGlobal>>
}
impl WlDisplay {
    /// Create a new Display with internal globals such as `wl_shm` registered
    /// 
    /// `formats` is the list of supported `wl_buffer` formats.
    pub fn new(formats: Vec<Format>) -> Self {
        let mut this = Self {
            serial: 0,
            globals: Vec::new()
        };
        this.register_global(WlShm::new(formats));
        this
    }
    /// Get a lease to the Display object
    #[inline(always)]
    pub fn get(client: &mut Client) -> Result<Lease<Self>> {
        client.get(Client::DISPLAY)
    }
    /// Register a global to be broadcast to the connecting client
    pub fn register_global<T: 'static + Global + Dispatch>(&mut self, global: T) {
        self.globals.push(Some(RegisteredGlobal {
            interface: T::INTERFACE,
            version: T::VERSION,
            global: Box::new(global)
        }))
    }
    /// Register a global to be broadcast to the connecting client with a specific version
    /// # Panics
    /// If the version is higher than that supported by the interface
    pub fn register_global_version<T: 'static + Global + Dispatch>(&mut self, version: u32, global: T) {
        if version > T::VERSION {
            panic!("The latest version of {:?} is {}, while the specified version {} exceeds that", T::INTERFACE, version, T::VERSION)
        }
        self.globals.push(Some(RegisteredGlobal {
            interface: T::INTERFACE,
            version,
            global: Box::new(global)
        }))
    }
    /// Get a value from and increment the wrapping counter
    pub fn serial(&mut self) -> u32 {
        self.serial = self.serial.wrapping_add(1);
        self.serial
    }
}
impl wayland::WlDisplay for Lease<WlDisplay> {
    fn sync(&mut self, client: &mut Client, callback: NewId) -> Result<()> {
        let mut lease = client.insert(callback, WlCallback)?;
        {
            use wayland::WlCallback;
            lease.done(client, self.serial())?;
            client.delete(&lease)?;
        }
        Ok(())
    }
    fn get_registry(&mut self, client: &mut Client, registry: NewId) -> Result<()> {
        use wayland::WlRegistry;
        let mut registry = client.insert(registry, WlRegistry)?;
        for (name, global) in self.globals.iter().enumerate() {
            if let Some(global) = global {
                registry.global(client, name as u32, global.interface, global.version)?
            }
        }
        Ok(())
    }
}

/// A short-lived object that simply signals its creation allowing a client to synchronize with the server
pub struct WlCallback;
impl wayland::WlCallback for Lease<WlCallback> {}

/// Manages the global objects providing access points to each supported interface
pub struct WlRegistry;
impl wayland::WlRegistry for Lease<WlRegistry> {
    fn bind(&mut self, client: &mut Client, global: u32, id: NewId) -> Result<()> {
        let mut display = WlDisplay::get(client)?;
        if let Some(global) = display.globals.get_mut(global as usize) {
            if let Some(global) = global {
                if global.interface != id.interface {
                    Err(DisplayError::method(self, "Attempted to bind to a global of a different interface".into()))
                } else if id.version > global.version {
                    Err(DisplayError::method(self, format!("Global {:?} supports up to version {} but version {} was requested", global.interface, global.version, id.version)))
                } else {
                    let lease = client.insert_any(id, global.global.construct())?;
                    let init = global.global.on_bind_fn();
                    init(lease, client, &mut display, self)
                }
            } else {
                // Ignore the request if the global is pending removal
                Ok(())
            }
        } else {
            Err(DisplayError::method(self, "No global by that name".into()))
        }
    }
}

macro_rules! wl_formats {
    (ARGB8888, XRGB8888$(, $format:ident)*) => {
        /// The format of a `wl_buffer`, a checked equivalent to `wayland::WlShmFormat`
        #[allow(non_camel_case_types)]
        #[derive(Copy, Clone)]
        pub enum Format {
            ARGB8888,
            XRGB8888,
            $($format),*
        }
        impl Format {
            pub fn new(object: &dyn Object, format: u32) -> Result<Self> {
                match format {
                    wayland::WlShmFormat::ARGB8888 => Ok(Self::ARGB8888),
                    wayland::WlShmFormat::XRGB8888 => Ok(Self::XRGB8888),
                    $(wayland::WlShmFormat::$format => Ok(Self::$format),)*
                    _ => Err(ShmError::InvalidFormat { object: object.object(), format }.into())
                }
            }
        }
        impl Into<u32> for Format {
            fn into(self) -> u32 {
                match self {
                    Self::ARGB8888 => wayland::WlShmFormat::ARGB8888,
                    Self::XRGB8888 => wayland::WlShmFormat::XRGB8888,
                    $(Self::$format => wayland::WlShmFormat::$format),*
                }
            }
        }
    };
}
wl_formats!{ARGB8888, XRGB8888, C8, RGB332, BGR233, XRGB4444, XBGR4444, RGBX4444, BGRX4444, ARGB4444, ABGR4444, RGBA4444, BGRA4444, XRGB1555, XBGR1555, RGBX5551, BGRX5551, ARGB1555, ABGR1555, RGBA5551, BGRA5551, RGB565, BGR565, RGB888, BGR888, XBGR8888, RGBX8888, BGRX8888, ABGR8888, RGBA8888, BGRA8888, XRGB2101010, XBGR2101010, RGBX1010102, BGRX1010102, ARGB2101010, ABGR2101010, RGBA1010102, BGRA1010102, YUYV, YVYU, UYVY, VYUY, AYUV, NV12, NV21, NV16, NV61, YUV410, YVU410, YUV411, YVU411, YUV420, YVU420, YUV422, YVU422, YUV444, YVU444, R8, R16, RG88, GR88, RG1616, GR1616, XRGB16161616F, XBGR16161616F, ARGB16161616F, ABGR16161616F, XYUV8888, VUY888, VUY101010, Y210, Y212, Y216, Y410, Y412, Y416, XVYU2101010, XVYU12_16161616, XVYU16161616, Y0L0, X0L0, Y0L2, X0L2, YUV420_8BIT, YUV420_10BIT, XRGB8888_A8, XBGR8888_A8, RGBX8888_A8, BGRX8888_A8, RGB888_A8, BGR888_A8, RGB565_A8, BGR565_A8, NV24, NV42, P210, P010, P012, P016, AXBXGXRX106106106106, NV15, Q410, Q401 }

error! {
    ShmError {
        InvalidFileDescriptor = wayland::WlShmError::INVALID_FD => fd {
            "invalid file descriptor, {reason}",
            reason: String
        },
        InvalidFormat = wayland::WlShmError::INVALID_FORMAT => format {
            "format {format} is unsupported", 
            format: u32
        },
        InvalidStride = wayland::WlShmError::INVALID_STRIDE => stride {
            "stride of {stride} is invalid",
            stride: u32
        }
    }
}

/// Global for creating new shared memory pools with which shared memory buffers can be associated.
/// 
/// Includes a list of buffer formats that are supported. `ARGB8888` and `XRGB8888` are always supported.
#[derive(Clone)]
pub struct WlShm {
    formats: Vec<Format>
}
impl WlShm {
    fn new(mut formats: Vec<Format>) -> Self {
        formats.push(Format::ARGB8888);
        formats.push(Format::XRGB8888);
        Self {
            formats
        }
    }
    fn bind(lease: Lease<dyn Any>, client: &mut Client, _: &mut Lease<WlDisplay>, _: &mut Lease<WlRegistry>) -> Result<()> {
        use wayland::WlShm;
        let mut lease: Lease<Self> = lease.downcast()?;
        for format in lease.formats.clone() {
            lease.format(client, format.into())?;
        }
        Ok(())
    }
}
impl Global for WlShm {
    fn construct(&self) -> Resident<dyn Any> {
        Client::reserve(self.clone())
    }
    fn on_bind_fn(&self) -> fn(Lease<dyn Any>, &mut Client, &mut Lease<WlDisplay>, &mut Lease<WlRegistry>) -> Result<()> {
        Self::bind
    }
}
impl wayland::WlShm for Lease<WlShm> {
    fn create_pool(&mut self, client: &mut Client, id: NewId, file: File, size: i32)-> Result<()>  {
        let pool = WlShmPool::new(&id, file, size)?;
        client.insert(id, pool)?;
        Ok(())
    }
}
/// A shared memory pool used by 0 or more buffers
struct ShmPool {
    buffer: *mut libc::c_void,
    size: usize,
    fd: i32,
    #[cfg(feature = "unsealed_shm")]
    did_fault: bool,
    #[cfg(feature = "unsealed_shm")]
    can_sigbus: bool
}
impl Drop for ShmPool {
    fn drop(&mut self) {
        use libc::*;
        unsafe {
            munmap(self.buffer as _, self.size);
            close(self.fd);
        }
    }
}
pub struct WlShmPool {
    pool: Rc<RefCell<ShmPool>>
}
impl WlShmPool {
    #[inline]
    pub fn new(object: &dyn Object, file: File, size: i32) -> Result<Self> {
        use std::os::unix::io::{AsRawFd, IntoRawFd};
        use libc::*;
        // Why is it a signed int?
        if size <= 0 {
            return Err(DisplayError::method(object, "the size of a wl_shm_pool must be bigger than 0".into()))
        }
        let fd = file.as_raw_fd();
        let size = size as usize;
        #[cfg(all(not(target_os = "linux"), not(feature = "unsealed_shm")))]
        compile_error!("Feature \"unsealed_shm\" requires file descriptor sealing which is only supported on Linux.");
        #[cfg(all(target_os = "linux", not(feature = "unsealed_shm")))]
        {
            // Ensure seals on the file are appropriate
            let seals = unsafe { fcntl(fd, F_GET_SEALS) };
            if seals == -1 || seals & F_SEAL_SHRINK == 0 {
                return Err(ShmError::fd(object, format!("file descriptor {} must allow sealing and have F_SEAL_SHRINK set to be used for shared memory", file.as_raw_fd())))
            }
        }
        #[cfg(all(not(target_os = "linux"), feature = "unsealed_shm"))]
        let can_sigbus = true;
        #[cfg(all(target_os = "linux", feature = "unsealed_shm"))]
        let can_sigbus = {
            // Ensure seals on the file are appropriate
            let seals = unsafe { fcntl(fd, F_GET_SEALS) };
            if seals != -1 && seals & F_SEAL_SHRINK != 0 {
                false
            } else {
                true
            }
        };

        let prot = PROT_READ | PROT_WRITE;
        let flags = MAP_SHARED;
        let buffer = unsafe { mmap(std::ptr::null_mut(), size, prot, flags, fd, 0) };
        if buffer == MAP_FAILED {
            let error = std::io::Error::last_os_error();
            let message = format!("Unable to mmap fd {}: {}", fd, error);
            return if error.raw_os_error().map(|e| e == ENOMEM).unwrap_or(false) {
                Err(DisplayError::out_of_memory(object, message))
            } else {
                Err(ShmError::fd(object, message))
            }
        }
        let pool = ShmPool {
            fd: file.into_raw_fd(),
            size,
            buffer,
            #[cfg(feature = "unsealed_shm")]
            did_fault: false,
            #[cfg(feature = "unsealed_shm")]
            can_sigbus
        };
        Ok(Self {
            pool: Rc::new(RefCell::new(pool))
        })
    }
    fn pool(&self) -> Result<std::cell::Ref<ShmPool>> {
        self.pool.try_borrow()
            .map_err(|e| SystemError::Other(Box::new(e)).into())
    }
    fn pool_mut(&self) -> Result<std::cell::RefMut<ShmPool>> {
        self.pool.try_borrow_mut()
            .map_err(|e| SystemError::Other(Box::new(e)).into())
    }
    #[cfg(feature = "unsealed_shm")]
    fn check_sigbus(object: &dyn Object, pool: &ShmPool) -> Result<()> {
        if pool.did_fault {
            return Err(ShmError::fd(object, "the shared memory file was shrunk".into())); 
        } else {
            Ok(())
        }
    }
}
impl wayland::WlShmPool for Lease<WlShmPool> {
    fn destroy(&mut self, client: &mut Client)-> Result<()>  {
        #[cfg(feature = "unsealed_shm")]
        WlShmPool::check_sigbus(self, &self.pool.borrow())?;
        client.delete(self)
    }
    fn create_buffer(&mut self, client: &mut Client, id: NewId, offset: i32, width: i32, height: i32, stride: i32, format: u32)-> Result<()>  {
        #[cfg(feature = "unsealed_shm")]
        WlShmPool::check_sigbus(self, &self.pool.borrow())?;
        let pool = self.pool()?;
        // Why are signed integers used?
        if offset < 0 || width <= 0 || height <= 0 || stride < width {
            return Err(DisplayError::method(&id, "Offset, stride, width or height is invalid".into()))
        }
        let (width, height, stride, offset) = (width as usize, height as usize, stride as usize, offset as usize);
        let format = Format::new(self, format)?;
        if pool.size < offset + stride * height {
            return Err(DisplayError::method(&id, "requested buffer would overflow available pool memory".into()))
        }
        std::mem::drop(pool);
        client.insert(id, WlBuffer::new(self.pool.clone(), offset, width, height, stride, format))?;
        Ok(())
    }
    fn resize(&mut self, _: &mut Client, size: i32)-> Result<()>  {
        #[cfg(feature = "unsealed_shm")]
        WlShmPool::check_sigbus(self, &self.pool.borrow())?;
        let mut pool = self.pool_mut()?;
        if size <= 0 {
            return Err(DisplayError::method(self, "the size of a wl_shm_pool must be bigger than 0".into()))
        }
        let size = size as usize;
        if size < pool.size {
            return Err(DisplayError::method(self, format!("cannot resize wl_shm_pool to be smaller. Previous size was {}b, requested is {}b", size, pool.size)))
        }
        use libc::*;
        let buffer = unsafe { mremap(pool.buffer, pool.size, size, MREMAP_MAYMOVE) };
        if buffer == MAP_FAILED {
            let error = std::io::Error::last_os_error();
            let message = format!("unable to resize mmapping for fd {}: {}", pool.fd, error);
            return if error.raw_os_error().map(|e| e == ENOMEM).unwrap_or(false) {
                Err(DisplayError::out_of_memory(self, message))
            } else {
                Err(ShmError::fd(self, message))
            }
        }
        pool.buffer = buffer as _;
        pool.size = size;
        Ok(())
    }
}
/// A shared memory buffer. Refers to a shared memory region in a shared memory pool.
pub struct WlBuffer {
    pool: Rc<RefCell<ShmPool>>,
    offset: usize,
    width: usize,
    height: usize,
    stride: usize,
    format: Format
}
impl WlBuffer {
    fn new(pool: Rc<RefCell<ShmPool>>, offset: usize, width: usize, height: usize, stride: usize, format: Format) -> Self {
        Self {
            pool,
            offset,
            width,
            height,
            stride,
            format
        }
    }
    #[cfg(feature = "unsealed_shm")]
    pub fn access<'a>(&'a self) -> Option<BufferAccess<'a>> {
        BufferAccess::new(self)
    }
    #[cfg(not(feature = "unsealed_shm"))]
    /// A pointer to the start of the buffer, valid until the pool is destroyed.
    /// 
    /// Lifetime information is discarded. It is unsafe to use this pointer after any operations on another WlBuffer or WlShmPool.
    /// The length of the pointed to array is defined by `WlBuffer::size`
    pub fn ptr(&self) -> *mut libc::c_void {
        (self.pool.borrow().buffer as usize + self.offset) as *mut _
    }
    /// The size of the buffer, in bytes
    pub fn size(&self) -> usize {
        self.offset + self.stride * self.height
    }
    pub fn offset(&self) -> usize {
        self.offset
    }
    pub fn width(&self) -> usize {
        self.width
    }
    pub fn height(&self) -> usize {
        self.height
    }
    pub fn stride(&self) -> usize {
        self.stride
    }
    pub fn format(&self) -> Format {
        self.format
    }
}
impl wayland::WlBuffer for Lease<WlBuffer> {
    fn destroy(&mut self, client: &mut Client)-> Result<()>  {
        #[cfg(feature = "unsealed_shm")]
        WlShmPool::check_sigbus(self, &self.pool.borrow())?;
        client.delete(self)
    }
}
/// Protects buffer access from SIGBUS signals caused due to the memory map being shrunk from under the compositor.
/// Only one buffer may be accessed per thread. Access is relinquished on drop.
#[cfg(feature = "unsealed_shm")]
#[repr(transparent)]
pub struct BufferAccess<'a>(&'a WlBuffer);
#[cfg(feature = "unsealed_shm")]
impl<'a> BufferAccess<'a> {
    fn new(buffer: &'a WlBuffer) -> Option<Self> {
        if buffer.pool.borrow().can_sigbus {
            if signal::sigbus_lock(buffer.pool.clone()) {
                Some(Self(buffer))
            } else {
                None
            }
        } else {
            Some(Self(buffer))
        }
    }
    /// A pointer to the start of the buffer, valid until the pool is destroyed.
    /// 
    /// Lifetime information is discarded. It is unsafe to use this pointer after any operations on another WlBuffer or WlShmPool.
    /// The length of the pointed to array is defined by `WlBuffer::size`
    pub fn ptr(&self) -> *mut libc::c_void {
        (self.0.pool.borrow().buffer as usize + self.0.offset) as *mut _
    }
}
#[cfg(feature = "unsealed_shm")]
impl<'a> std::ops::Deref for BufferAccess<'a> {
    type Target = WlBuffer;
    fn deref(&self) -> &Self::Target {
        self.0
    }
}
#[cfg(feature = "unsealed_shm")]
impl<'a> Drop for BufferAccess<'a> {
    fn drop(&mut self) {
        if self.0.pool.borrow().can_sigbus {
            signal::sigbus_unlock()
        }
    }
}