use std::{fmt, fs::File, any::Any, rc::Rc, cell::RefCell};

use wl::{server::prelude::*, SystemError};
pub use prelude::Global;

pub mod prelude {
    /// Derive macro to simplify the implementation of Global interfaces
    pub use global_derive_macro::ServerGlobal as Global;
    pub use super::{
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
/// A server error type representing core wayland errors that will be signalled to the client
pub struct DisplayError {
    object: u32,
    code: u32,
    message: String
}
impl DisplayError {
    pub fn method(object: &dyn Object, message: String) -> Error {
        Error::Protocol(Box::new(Self {
            object: object.object(),
            code: wayland::WlDisplayError::INVALID_METHOD,
            message
        }))
    }
    pub fn out_of_memory(object: &dyn Object, message: String) -> Error {
        Error::Protocol(Box::new(Self {
            object: object.object(),
            code: wayland::WlDisplayError::NO_MEMORY,
            message
        }))
    }
}
impl ErrorHandler for DisplayError {
    fn handle(&mut self, client: &mut Client) -> Result<()> {
        let mut display = WlDisplay::get(client)?;
        {
            use wayland::WlDisplay;
            display.error(client, &self.object, self.code, &self.message)
        }
    }
}
impl fmt::Display for DisplayError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        use wayland::WlDisplayError;
        match self.code {
            WlDisplayError::IMPLEMENTATION => write!(f, "Internal error within the compositor, {}", self.message),
            WlDisplayError::NO_MEMORY => write!(f, "The compositor is out of memory, {}", self.message),
            WlDisplayError::INVALID_OBJECT => write!(f, "The requested object was not found, {}", self.message),
            WlDisplayError::INVALID_METHOD => write!(f, "Malformed request could not be processed, {}", self.message),
            _ => write!(f, "Unknown error on object {}, {}", self.object, self.message)
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
            pub fn new(format: u32) -> Result<Self> {
                match format {
                    wayland::WlShmFormat::ARGB8888 => Ok(Self::ARGB8888),
                    wayland::WlShmFormat::XRGB8888 => Ok(Self::XRGB8888),
                    $(wayland::WlShmFormat::$format => Ok(Self::$format),)*
                    _ => Err(DisplayError::method(&0, format!("No wl_shm format for value {}", format)))
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
    fd: i32
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
            return Err(DisplayError::method(object, "The size of a wl_shm_pool must be bigger than 0.".into()))
        }
        let fd = file.as_raw_fd();
        let size = size as usize;
        // Ensure seals on the file are appropriate
        let seals = unsafe { fcntl(fd, F_GET_SEALS) };
        #[cfg(not(feature = "unsealed_shm"))]
        if seals == -1 {
            return Err(DisplayError::method(object, format!("File descriptor {} must support sealing and have F_SEAL_SHRINK set.", fd)));
        } else if seals & F_SEAL_SHRINK == 0 {
            return Err(DisplayError::method(object, format!("File descriptor {} must have shrink operations sealed.", fd)));
        }
        #[cfg(feature = "unsealed_shm")]
        compile_error!("TODO: implement");

        let prot = PROT_READ | PROT_WRITE;
        let flags = MAP_SHARED;
        let buffer = unsafe { mmap(std::ptr::null_mut(), size, prot, flags, fd, 0) };
        if buffer == MAP_FAILED {
            let error = std::io::Error::last_os_error();
            let message = format!("Unable to mmap fd {}: {}", fd, error);
            return if error.raw_os_error().map(|e| e == ENOMEM).unwrap_or(false) {
                Err(DisplayError::out_of_memory(object, message))
            } else {
                Err(DisplayError::method(object, message))
            }
        }
        let pool = ShmPool {
            fd: file.into_raw_fd(),
            size,
            buffer
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
}
impl wayland::WlShmPool for Lease<WlShmPool> {
    fn destroy(&mut self, client: &mut Client)-> Result<()>  {
        client.delete(self)
    }
    fn create_buffer(&mut self, client: &mut Client, id: NewId, offset: i32, width: i32, height: i32, stride: i32, format: u32)-> Result<()>  {
        let pool = self.pool()?;
        // Why are signed integers used?
        if offset < 0 || width <= 0 || height <= 0 || stride < width {
            return Err(DisplayError::method(&id, "Offset, stride, width or height is invalid".into()))
        }
        let (width, height, stride, offset) = (width as usize, height as usize, stride as usize, offset as usize);
        let format = Format::new(format)?;
        if pool.size < offset + stride * height {
            return Err(DisplayError::method(&id, "Requested buffer would overflow available pool memory".into()))
        }
        std::mem::drop(pool);
        client.insert(id, WlBuffer::new(self.pool.clone(), offset, width, height, stride, format))?;
        Ok(())
    }
    fn resize(&mut self, _: &mut Client, size: i32)-> Result<()>  {
        let mut pool = self.pool_mut()?;
        if size <= 0 {
            return Err(DisplayError::method(self, "The size of a wl_shm_pool must be bigger than 0.".into()))
        }
        let size = size as usize;
        if size < pool.size {
            return Err(DisplayError::method(self, format!("Cannot resize wl_shm_pool to be smaller. Previous size was {}b, requested is {}b.", size, pool.size)))
        }
        use libc::*;
        let buffer = unsafe { mremap(pool.buffer, pool.size, size, MREMAP_MAYMOVE) };
        if buffer == MAP_FAILED {
            let error = std::io::Error::last_os_error();
            let message = format!("Unable to resize mmapping for fd {}: {}", pool.fd, error);
            return if error.raw_os_error().map(|e| e == ENOMEM).unwrap_or(false) {
                Err(DisplayError::out_of_memory(self, message))
            } else {
                Err(DisplayError::method(self, message))
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
    /// A pointer to the start of the buffer, valid until the pool is destroyed.
    /// 
    /// Lifetime information is discarded. It is unsafe to use this pointer after any operations on another WlBuffer or WlShmPool.
    /// The length of the pointed to array is defined by `WlBuffer::size`
    pub fn ptr(&self) -> *mut std::ffi::c_void {
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
        client.delete(self)
    }
}