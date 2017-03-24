use jack_sys;
use libc;

use std::ffi::{CString, CStr};

use callbackhandler::*;
use midi::*;
use port::*;
use types::*;

/// A jack client connected to a jack server
///
/// TODO example
pub struct Client<'a> {
    c_client: *mut jack_sys::jack_client_t,

    // store the handlers in a box so that we can store a trait object + take ownership
    // I do not like boxing everything up because it causes unnecessary heap allocation :(
    process_handler:  Option<Box<ProcessHandler + 'a>>,
    metadata_handler: Option<Box<MetadataHandler + 'a>>
}

impl<'a> Client<'a> {
    fn open_helper(cl: *mut jack_sys::jack_client_t, status: u32, name: &str)
        -> Result<(Self, String), status::Status>
    {
        let status = status::Status::from_bits(status).unwrap();
        if cl.is_null() {
            Err(status)
        } else {
            let cl = Client {
                c_client:          cl,
                process_handler:   None,
                metadata_handler:  None,
            };

            let name = if status.contains(status::NAME_NOT_UNIQUE) {
                cl.get_name()
            } else {
                name.to_string()
            };

            Ok( (cl, name) )
        }
    }

    /// Creates a new client and connects it to the default jack server. The
    /// client will use the name given. If the name is not unique, the behavior
    /// depends on the options provided via `opts`.
    ///
    /// If the option to force a unique name is given (USE_EXACT_NAME) and the exact name can not
    /// be given, Err will be returned. Otherwise Returns the client and the name assigned to the
    /// client.
    ///
    /// TODO client_name_size details in docs and in code
    pub fn open(name: &str, opts: options::Options) -> Result<(Self, String), status::Status> {
        // TODO does jack check if the options are valid?
        // TODO does jack check if the name is too large?

        let cstr       = CString::new(name).unwrap();
        let mut status = 0 as jack_sys::jack_status_t;
        let statusptr  = &mut status as *mut jack_sys::jack_status_t;

        let cl = unsafe { jack_sys::jack_client_open(cstr.as_ptr(), opts.bits(), statusptr) };
        Client::open_helper(cl, status, name)
    }

    /// Attempts to open a client connecting to a server with a specified name
    pub fn open_connection_to(
        clientname: &str,
        servername: &str,
        opts: options::Options)
        -> Result<(Self, String), status::Status>
    {
        let cstr       = CString::new(clientname).unwrap();
        let sstr       = CString::new(servername).unwrap();
        let mut status = 0 as jack_sys::jack_status_t;
        let statusptr  = &mut status as *mut jack_sys::jack_status_t;

        let additionalopts = options::Options::from_bits(jack_sys::JackServerName).unwrap();
        let cl = unsafe {
            jack_sys::jack_client_open(
                cstr.as_ptr(),
                (opts | additionalopts).bits(),
                statusptr,
                sstr.as_ptr())
        };

        Client::open_helper(cl, status, clientname)
    }

    /// Returns the actual name of the client. This is useful when
    /// USE_EXACT_NAME is not specified, because the jack server might assign
    /// some other name to your client to ensure that it is unique.
    ///
    /// Returns a copy of the actual string returned the JACK C API
    pub fn get_name(&self) -> String {
        // use jack's getters and setters because the names are subject to change
        // do not need to free the string
        unsafe {
            let raw = self.c_client;
            let cstr = jack_sys::jack_get_client_name(raw);
            String::from(CStr::from_ptr(cstr).to_str().unwrap())
        }
    }

    /// Create a new port for this client. Ports are used to move data in and out of the client
    /// (audio data, midi data, etc). Ports may be connected to other ports in various ways.
    ///
    /// Each port has a short name which must be unique among all the ports owned by the client.
    /// The port's full name contains the name of the client, followed by a colon (:), followed by
    /// the port's short name.
    ///
    /// All ports have a type. The `port_type` module contains port types which may be used.
    ///
    /// You may also specify a number of flags from the `port_flags` module which control the
    /// behavior of the created port (input vs output, etc)
    ///
    /// This function has to figure out what kind of port to return based on the flags provided.
    ///
    /// TODO something about buffer size I haven't figured out yet
    /// TODO port_name_size()
    fn register_port(
        &mut self,
        name: &str,
        ptype: PortType,
        opts: port_flags::PortFlags)
        -> Result<UnknownPortHandle, status::Status>
    {
        let cstr = CString::new(name).unwrap();
        let typestr = CString::new(ptype).unwrap();

        let port = unsafe {
            jack_sys::jack_port_register(
                self.c_client,
                cstr.as_ptr(),
                typestr.as_ptr(),
                opts.bits() as ::libc::c_ulong,
                0)
        };

        if port.is_null() {
            // no error code is returned from jack here
            Err(status::FAILURE)
        } else {
            Ok(UnknownPortHandle::new(port))
        }
    }

    /// Helper function which registers an input audio port with a given name.
    pub fn register_input_audio_port(&mut self, name: &str)
            -> Result<InputPortHandle<DefaultAudioSample>, status::Status>
    {
        let p = self.register_port(
            name,
            port_type::DEFAULT_AUDIO_TYPE,
            port_flags::PORT_IS_INPUT);

        p.map(|p| unsafe { p.force_as_input::<DefaultAudioSample>() })
    }

    /// Helper function which registers an input midi port with a given name.
    pub fn register_input_midi_port(&mut self, name: &str)
            -> Result<InputPortHandle<MidiEvent>, status::Status>
    {
        let p = self.register_port(
            name,
            port_type::DEFAULT_MIDI_TYPE,
            port_flags::PORT_IS_INPUT);

        p.map(|p| unsafe { p.force_as_input::<MidiEvent>() })
    }

    /// Helper function which registers an output audio port with a given name.
    pub fn register_output_audio_port(&mut self, name: &str)
            -> Result<OutputPortHandle<DefaultAudioSample>, status::Status>
    {
        let p = self.register_port(
            name,
            port_type::DEFAULT_AUDIO_TYPE,
            port_flags::PORT_IS_OUTPUT);

        p.map(|p| unsafe { p.force_as_output::<DefaultAudioSample>() })
    }

    /// Removes the port from the client and invalidates the port and all
    /// Handles relating to the port.
    ///
    /// The server disconnects everything that was previously connected to the port.
    pub fn unregister_port<T: Port>(&mut self, port: T) -> Result<(), status::Status> {
        let ret = unsafe { jack_sys::jack_port_unregister(self.c_client, port.get_raw()) };

        if ret == 0 {
            Ok(())
        } else {
            // TODO try to handle this error code
            Err(status::FAILURE)
        }
    }

    pub fn get_port_by_name(&self, name: &str) -> Option<UnknownPortHandle> {
        let cstr = CString::new(name).unwrap();
        let ptr = unsafe { jack_sys::jack_port_by_name(self.c_client, cstr.as_ptr()) };

        if ptr.is_null() {
            None
        } else {
            Some(UnknownPortHandle::new(ptr))
        }
    }

    pub fn get_port_by_id(&self, id: PortId) -> Option<UnknownPortHandle> {
        let ptr = unsafe { jack_sys::jack_port_by_id(self.c_client, id) };

        if ptr.is_null() {
            None
        } else {
            Some(UnknownPortHandle::new(ptr))
        }
    }

    /// Attempts to connect the ports with the given names
    /// Note that this method calls directly into the jack api. It does not
    /// perform lookups for the names before making the call
    pub fn connect_ports(&mut self, port1: &str, port2: &str) -> Result<(), status::Status> {
        let res = unsafe {
            jack_sys::jack_connect(
                self.c_client,
                CString::new(port1).unwrap().as_ptr(),
                CString::new(port2).unwrap().as_ptr())
        };

        if res == 0 {
            Ok(())
        } else {
            // TODO figure out what these error codes mean
            println!("error code: {}", res);
            Err(status::FAILURE)
        }
    }


    /// Attempts to disconnect the ports with the given names
    /// Note that this method calls directly into the jack api. It does not
    /// perform lookups for the names before making the call
    pub fn disconnect_ports(&mut self, port1: &str, port2: &str) -> Result<(), status::Status> {
        let res = unsafe {
            jack_sys::jack_disconnect(
                self.c_client,
                CString::new(port1).unwrap().as_ptr(),
                CString::new(port2).unwrap().as_ptr())
        };

        if res == 0 {
            Ok(())
        } else {
            Err(status::Status::from_bits(res as u32).unwrap())
        }
    }

    /// Set the client's process callback handler.
    /// The client takes ownership of the handler, so be sure to set up any
    /// messaging queues before passing the handler off to the client
    /// See the docs for the `ProcessHandler` struct for more details
    pub fn set_process_handler<T: ProcessHandler + 'a>(&mut self, handler: T)
        -> Result<(), status::Status>
    {
        // a function which will do some setup then call the client's handler
        // this function must be generic over <T>.
        // Trait pointers are "fat pointers" (not raw pointers), so we can't
        // pass trait pointers around via a C void*
        unsafe extern "C" fn process_callback<T: ProcessHandler>(
            nframes: jack_sys::jack_nframes_t,
            args: *mut libc::c_void)
            -> libc::c_int
        {
            let this = args as *mut T;
            let ctx = CallbackContext::new();
            (*this).process(&ctx, nframes)
        }

        // create a box for this handler
        // this will allocate memory and move the object to the allocated memory
        // on the heap
        let b = Box::new(handler);

        // get the pointer, this consumes the box, but does not move the
        // resulting memory anywhere
        let ptr = Box::into_raw(b);

        let ret = unsafe {
            let ptr = ptr as *mut libc::c_void;
            jack_sys::jack_set_process_callback(
                self.c_client, Some(process_callback::<T>), ptr)
        };

        if ret != 0 {
            // again, no error code provided
            Err(status::FAILURE)
        } else {
            // create a box from the raw pointer. this does not allocate more memory
            let b = unsafe { Box::from_raw(ptr) };
            self.process_handler = Some(b);
            Ok(())
        }
    }

    /// Set the client's metadata change handler.
    pub fn set_metadata_handler<T: MetadataHandler + 'a>(&mut self, handler: T)
        -> Result<(), status::Status>
    {
        unsafe extern "C" fn srate_callback<T: MetadataHandler>(
            srate: NumFrames,
            args: *mut libc::c_void) -> i32
        {
            let this = args as *mut T;

            (*this).sample_rate_changed(srate)
        }

        unsafe extern "C" fn connect_callback<T: MetadataHandler>(
            a: jack_sys::jack_port_id_t,
            b: jack_sys::jack_port_id_t,
            connect: libc::c_int,
            args: *mut libc::c_void)
        {
            let this = args as *mut T;
            let status = if connect == 0 {
                PortConnectStatus::PortsDisconnected
            } else {
                PortConnectStatus::PortsConnected
            };

            (*this).on_port_connect(a, b, status)
        }

        unsafe extern "C" fn xrun_callback<T: MetadataHandler>(
            args: *mut libc::c_void) -> i32
        {
            let this = args as *mut T;
            (*this).on_xrun()
        }

        let b = Box::new(handler);
        let cbs = b.callbacks_of_interest();

        let ptr = Box::into_raw(b);

        let ret = unsafe {
            let ptr = ptr as *mut libc::c_void;

            let mut ret = 0;
            for h in cbs {
                ret = match h {
                    MetadataHandlers::SampleRate =>
                        jack_sys::jack_set_sample_rate_callback(
                            self.c_client, Some(srate_callback::<T>), ptr),

                    MetadataHandlers::PortConnect =>
                        jack_sys::jack_set_port_connect_callback(
                            self.c_client, Some(connect_callback::<T>), ptr),

                    // MetadataHandlers::Shutdown
                    // MetadataHandlers::Freewheel,
                    // MetadataHandlers::BufferSize,
                    // MetadataHandlers::ClientRegistration,
                    // MetadataHandlers::PortRegistration,
                    // MetadataHandlers::PortRename,
                    // MetadataHandlers::GraphOrder,

                     MetadataHandlers::Xrun =>
                         jack_sys::jack_set_xrun_callback(
                             self.c_client, Some(xrun_callback::<T>), ptr),

                    _  => unimplemented!(),
                };

                if ret != 0 {
                    break;
                }
            }

            ret
        };

        if ret != 0 {
            // again, no error code provided
            Err(status::FAILURE)
        } else {
            // create a box from the raw pointer. this does not allocate more memory
            let b = unsafe { Box::from_raw(ptr) };
            self.metadata_handler = Some(b);
            Ok(())
        }
    }

    /// tells the JACK server that the client is read to start processing audio
    /// This will initiate
    /// callbacks into the `CallbackHandler` provided.
    pub fn activate(&self) -> Result<(), status::Status> {
        // TODO disable various other function calls after activate is called
        // do this via (self) -> ActivatedClient or something
        let ret = unsafe { jack_sys::jack_activate(self.c_client) };

        if ret != 0 {
            // TODO handle error
            Err(status::FAILURE)
        } else {
            Ok(())
        }
    }

    /// Disconnects the client from the JACK server.
    /// This will also disconnect and destroy any of the ports which the client registered
    pub fn close(&mut self) -> Result<(), &str> {
        let ret = unsafe { jack_sys::jack_client_close(self.c_client) };

        if ret == 0 {
            Ok(())
        } else {
            Err("some error should go here")
        }
    }

    #[cfg(test)]
    pub unsafe fn get_raw(&self) -> *const jack_sys::jack_client_t { self.c_client }
}

// these tests are extremely fragile because they involve using a c library as the stub mechanism
#[cfg(test)]
mod test {
    extern crate libc;

    use super::*;
    use types::*;
    use jack_sys;

    use std::ffi::*;
    use std::ptr;

    // statically link the wrapper in
    // this will overwrite all of the jack symbols + add our new additional ones for our stub
    // all of these functions are thread safe (we use thread local storage)
    #[link(name="jack_wrapper", kind="static")]
    extern "C" {
        // jack_client_open
        pub fn jco_set_return(ptrval: *mut jack_sys::jack_client_t);
        pub fn jco_set_status_return(status: libc::c_uint);
        pub fn jco_get_passed_client_name() -> *const libc::c_char;
        pub fn jco_get_passed_server_name() -> *const libc::c_char;
        pub fn jco_get_passed_options() -> libc::c_uint;
        pub fn jco_get_num_calls() -> libc::size_t;
        pub fn jco_setup();
        pub fn jco_cleanup();

        // jack_get_client_name
        pub fn jgcn_set_return(name: *const libc::c_char);
        pub fn jgcn_get_passed_client() -> *mut jack_sys::jack_client_t;
        pub fn jgcn_get_num_calls() -> libc::size_t;
        pub fn jgcn_setup();
        pub fn jgcn_cleanup();
    }

    struct JackClientOpen { }
    impl JackClientOpen { pub fn setup() { unsafe { jco_setup(); } } }
    impl Drop for JackClientOpen { fn drop(&mut self) { unsafe { jco_cleanup(); } } }

    struct JackGetClientName { }
    impl JackGetClientName { pub fn setup() { unsafe { jgcn_setup(); } } }
    impl Drop for JackGetClientName { fn drop(&mut self) { unsafe { jgcn_cleanup(); } } }

    #[test]
    fn test_client_open_fail() {
        let _jco = JackClientOpen::setup();

        // test that open will fail if the jack method returns null
        // checking only one case with status bits because these could be any number of things
        // depending on the state of the server
        unsafe {
            jco_set_return(ptr::null_mut());
            jco_set_status_return(status::INIT_FAILURE.bits());
        };

        let client = Client::open("test", options::NO_START_SERVER);
        assert!(client.is_err());
        assert!(client.err().unwrap() == status::INIT_FAILURE);

        assert!(unsafe { jco_get_num_calls() } == 1);
    }

    #[test]
    fn test_client_open_okay() {
        let (_co, _gn) = (JackClientOpen::setup(), JackGetClientName::setup());

        // test that open will succeed if the jack method returns a valid pointer
        let ptr = 0xdeadbeef as *mut jack_sys::jack_client_t;
        unsafe { jco_set_return(ptr) };

        let client = Client::open("test", options::NO_START_SERVER);
        let used_name = unsafe { CStr::from_ptr(jco_get_passed_client_name()) };

        let opts = unsafe { jco_get_passed_options() };
        let opts = options::Options::from_bits(opts);

        assert!(used_name.to_str().unwrap() == "test");
        assert!(client.is_ok());
        assert!(opts.is_some());
        assert!(opts.unwrap() == options::NO_START_SERVER);

        let server_name = unsafe { jco_get_passed_server_name() };
        assert!(server_name.is_null());

        unsafe {
            assert!(client.unwrap().0.get_raw() == ptr);
            assert!(jco_get_num_calls() == 1);

            // should not attempt to call get_name (potentially expensive)
            assert!(jgcn_get_num_calls() == 0);
        }
    }

    #[test]
    fn test_client_name_not_unique_succ() {
        let (_co, _gn) = (JackClientOpen::setup(), JackGetClientName::setup());

        let ptr = 0xdeadbeef as *mut jack_sys::jack_client_t;
        unsafe {
            jco_set_status_return(status::NAME_NOT_UNIQUE.bits());
            jco_set_return(ptr);
        };

        let name = CString::new("notunique.1").unwrap();
        unsafe {
            jgcn_set_return(name.as_ptr());
        }

        let client = Client::open("notunique", options::NO_START_SERVER);

        let used_name = unsafe { CStr::from_ptr(jco_get_passed_client_name()) };
        assert!(used_name.to_str().unwrap() == "notunique");
        assert!(unsafe { jgcn_get_passed_client() } == ptr);

        assert!(client.is_ok());
        let client = client.unwrap();
        assert!(unsafe { client.0.get_raw() } == ptr);
        assert!(client.1 == "notunique.1");

        let opts = unsafe { jco_get_passed_options() };
        let opts = options::Options::from_bits(opts);
        assert!(opts.is_some());
        assert!(opts.unwrap() == options::NO_START_SERVER);

        unsafe {
            assert!(jco_get_num_calls() == 1);
            assert!(jgcn_get_num_calls() == 1);
        }
    }

    #[test]
    fn test_client_name_not_unique_fail() {
        let (_co, _gn) = (JackClientOpen::setup(), JackGetClientName::setup());

        unsafe {
            jco_set_status_return(status::NAME_NOT_UNIQUE.bits()); // TODO verify the return
            jco_set_return(ptr::null_mut());
        };

        let client = Client::open("notunique",
                                  options::NO_START_SERVER & options::USE_EXACT_NAME);
        assert!(client.is_err());

        let used_name = unsafe { CStr::from_ptr(jco_get_passed_client_name()) };
        assert!(used_name.to_str().unwrap() == "notunique");

        unsafe {
            assert!(jco_get_num_calls() == 1);
            assert!(jgcn_get_num_calls() == 0);
        }
    }

    #[test]
    fn open_connection_to_named_server_okay() {
        let _jco = JackClientOpen::setup();

        // test that open will succeed if the jack method returns a valid pointer
        let ptr = 0xdeadbeef as *mut jack_sys::jack_client_t;
        unsafe { jco_set_return(ptr) };

        let client = Client::open_connection_to("client", "server", options::NO_START_SERVER);
        let used_client_name = unsafe { CStr::from_ptr(jco_get_passed_client_name()) };
        let used_server_name = unsafe { CStr::from_ptr(jco_get_passed_server_name()) };

        assert!(client.is_ok());
        assert!(used_client_name.to_str().unwrap() == "client");
        assert!(used_server_name.to_str().unwrap() == "server");

        let opts = unsafe { jco_get_passed_options() };
        let opts = options::Options::from_bits(opts);
        assert!(opts.is_some());
        assert!(opts.unwrap() == options::NO_START_SERVER | options::SERVER_NAME);

        unsafe {
            assert!(client.unwrap().0.get_raw() == ptr);
            assert!(jco_get_num_calls() == 1);
        }
    }

    #[test]
    fn open_connection_to_named_server_fail() {
        let _jco = JackClientOpen::setup();

        // only testing that this method returns an error when the function returns a null
        // checking only one case with status bits, these could be any number of things
        unsafe {
            jco_set_return(ptr::null_mut());
            jco_set_status_return(status::INIT_FAILURE.bits());
        };

        let client = Client::open_connection_to("client", "server", options::NO_START_SERVER);
        assert!(client.is_err());
        assert!(client.err().unwrap() == status::INIT_FAILURE);

        assert!(unsafe { jco_get_num_calls() } == 1);
    }
}
