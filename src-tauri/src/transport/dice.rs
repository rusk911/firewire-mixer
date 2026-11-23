use std::thread::{self, JoinHandle};

use firewire_dice_protocols::tcat::{
    global_section::{GlobalParameters, TcatGlobalSectionSpecification},
    GeneralSections, TcatMutableSectionOperation, TcatOperation, TcatSectionOperation,
};
use glib::{MainContext, MainLoop, Source};
use hinawa::prelude::FwNodeExt;
use hinawa::{FwNode, FwReq};

const TRANSACTION_TIMEOUT_MS: u32 = 200;

pub struct DiceTransport {
    node: FwNode,
    req: FwReq,
    _context: MainContext,
    main_loop: MainLoop,
    _source: Source,
    dispatcher: Option<JoinHandle<()>>,
}

impl DiceTransport {
    pub fn open(node_path: &str) -> Result<Self, String> {
        if !is_firewire_node_path(node_path) {
            return Err(format!("Invalid FireWire device path: {node_path}"));
        }

        let node = FwNode::new();
        node.open(node_path, 0)
            .map_err(|error| format!("Failed to open {node_path}: {error}"))?;

        let context = MainContext::new();
        let source = node
            .create_source()
            .map_err(|error| format!("Failed to create FireWire event source: {error}"))?;
        source.attach(Some(&context));

        let main_loop = MainLoop::new(Some(&context), false);
        let dispatch_loop = main_loop.clone();
        let dispatcher = thread::Builder::new()
            .name("firemix-dice-events".to_owned())
            .spawn(move || dispatch_loop.run())
            .map_err(|error| format!("Failed to start GLib event dispatcher: {error}"))?;

        Ok(Self {
            node,
            req: FwReq::new(),
            _context: context,
            main_loop,
            _source: source,
            dispatcher: Some(dispatcher),
        })
    }

    pub fn node(&self) -> &FwNode {
        &self.node
    }

    pub fn request(&self) -> &FwReq {
        &self.req
    }

    pub fn read_global_section(&self) -> Result<(GeneralSections, GlobalParameters), String> {
        let mut sections = GeneralSections::default();
        <Self as TcatOperation>::read_general_sections(
            &self.req,
            &self.node,
            &mut sections,
            TRANSACTION_TIMEOUT_MS,
        )
        .map_err(|error| format!("Failed to read DICE section table: {error}"))?;

        let mut params = GlobalParameters::default();
        <Self as TcatSectionOperation<GlobalParameters>>::whole_cache(
            &self.req,
            &self.node,
            &sections.global,
            &mut params,
            TRANSACTION_TIMEOUT_MS,
        )
        .map_err(|error| format!("Failed to read DICE Global Section: {error}"))?;

        Ok((sections, params))
    }

    pub fn write_global_section(
        &self,
        sections: &GeneralSections,
        params: &GlobalParameters,
        previous: &mut GlobalParameters,
    ) -> Result<(), String> {
        <Self as TcatMutableSectionOperation<GlobalParameters>>::partial_update(
            &self.req,
            &self.node,
            &sections.global,
            params,
            previous,
            TRANSACTION_TIMEOUT_MS,
        )
        .map_err(|error| format!("Failed to write DICE Global Section: {error}"))
    }
}

impl TcatOperation for DiceTransport {}

impl TcatGlobalSectionSpecification for DiceTransport {}

impl Drop for DiceTransport {
    fn drop(&mut self) {
        self.main_loop.quit();
        if let Some(dispatcher) = self.dispatcher.take() {
            let _ = dispatcher.join();
        }
    }
}

fn is_firewire_node_path(path: &str) -> bool {
    path.strip_prefix("/dev/fw")
        .is_some_and(|node| !node.is_empty() && node.bytes().all(|byte| byte.is_ascii_digit()))
}