use jack_sys;
use libc;

use std::ffi::{CString, CStr};

use callbackhandler::*;
use midi::*;
use port::*;
use types::*;

/// A jack client connected to a jack server
/// TODO example
/// TODO multi_handler_registration - we need a way to register one struct as a handler for
/// multiple callbacks, else this will become a giant mess very quickly
pub struct Client<'a> {
    c_client: *mut jack_sys::jack_client_t,

    // store the handlers in a box so that we can store a trait object + take ownership
    // I do not like boxing everything up because it causes unnecessary heap allocation :(
    process_handler:  Option<Box<ProcessHandler + 'a>>,
    metadata_handler: Option<Box<MetadataHandler + 'a>>
}

impl<'a> Client<'a> {
    /// Creates a new client and connects it to the default jack server. The
    /// client will use the name given. If the name is not unique, the behavior
    /// depends on the options provided via `opts`
    ///
    /// TODO this interface is not entirely correct, fix it. There is potential
    /// for a status to be returned even if the creation fails
    ///
    /// TODO client_name_size details in docs and in code
    pub fn open(name: &str, opts: options::Options) -> Result<Self, status::Status> {
        // TODO does jack check if the options are valid?
        let cstr = CString::new(name).unwrap();
        let mut status = 0 as jack_sys::jack_status_t;
        let statusptr = &mut status as *mut jack_sys::jack_status_t;

        let cl = unsafe { jack_sys::jack_client_open(cstr.as_ptr(), opts.bits(), statusptr) };

        if cl.is_null() {
            // if this fails, we are accepting a potential panic
            let status = status::Status::from_bits(status).unwrap();
            Err(status)
        } else {
            // TODO check status anyway
            Ok(Client {
                c_client:          cl,
                process_handler:   None,
                metadata_handler:  None,
            })
        }
    }

    /// TODO document this
    pub fn open_connection_to(
        clientname: &str,
        servername: &str,
        opts: options::Options)
        -> Result<Self, status::Status>
    {
        let cstr = CString::new(clientname).unwrap();
        let sstr = CString::new(servername).unwrap();
        let mut status = 0 as jack_sys::jack_status_t;
        let statusptr = &mut status as *mut jack_sys::jack_status_t;

        let additionalopts = options::Options::from_bits(jack_sys::JackServerName).unwrap();
        let cl = unsafe {
            jack_sys::jack_client_open(
                cstr.as_ptr(),
                (opts | additionalopts).bits(),
                statusptr,
                sstr.as_ptr())
        };

        if cl.is_null() {
            // if this fails, we are accepting a potential panic
            let status = status::Status::from_bits(status).unwrap();
            Err(status)
        } else {
            // TODO check status anyway
            Ok(Client {
                c_client:          cl,
                process_handler:   None,
                metadata_handler:  None,
            })
        }

    }

    /// Returns the actual name of the client. This is useful when
    /// USE_EXACT_NAME is not specified, because the jack server might assign
    /// some other name to your client to ensure that it is unique.
    pub fn get_name(&self) -> &str {
        // use jack's getters and setters because the names are subject to change
        // do not need to free the string
        unsafe {
            let raw = self.c_client;
            let cstr = jack_sys::jack_get_client_name(raw);
            CStr::from_ptr(cstr).to_str().unwrap()
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
                opts.bits() as u64,
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
        extern "C" fn process_callback<T: ProcessHandler>(
            nframes: jack_sys::jack_nframes_t,
            args: *mut libc::c_void)
            -> libc::c_int
        {
            let this = args as *mut T;
            let ctx = CallbackContext::new();
            unsafe { (*this).process(&ctx, nframes) }
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

    /// Set the client's sample rate change handler.
    pub fn set_metadata_handler<T: MetadataHandler + 'a>(&mut self, handler: T)
        -> Result<(), status::Status>
    {
        extern "C" fn srate_callback<T: MetadataHandler>(
            srate: NumFrames,
            args: *mut libc::c_void) -> i32
        {
            let this = args as *mut T;

            unsafe { (*this).sample_rate_changed(srate) }
        }

        extern "C" fn connect_callback<T: MetadataHandler>(
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

            unsafe { (*this).on_port_connect(a, b, status) }
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
                    // MetadataHandlers::Xrun,
                    _           => unimplemented!(),
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
}
