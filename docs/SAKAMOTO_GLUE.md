# How to Glue `hinawa` to `firewire-*-protocols`

## 1. DICE Glue (`TcatOperation` trait)
`firewire-dice-protocols` does not depend on `hinawa`. To use it, implement `TcatOperation` on a wrapper holding `hinawa::FwNode` and `hinawa::FwReq`:

```rust
use hinawa::{FwNode, FwReq, FwReqExtManual, FwTcode};
use firewire_dice_protocols::tcat::{TcatOperation, TcatNotified};
use glib::Error;

pub struct DiceTransport {
    pub node: FwNode,
    pub req: FwReq,
}

impl TcatOperation for DiceTransport {
    fn read(&mut self, addr: usize, frames: &mut [u32], timeout_ms: u32) -> Result<(), Error> {
        let mut raw = vec![0u8; frames.len() * 4];
        let tcode = if frames.len() == 1 {
            FwTcode::ReadQuadletRequest
        } else {
            FwTcode::ReadBlockRequest
        };
        // DICE base address is 0xffffe0000000
        self.req.transaction_sync(
            &self.node,
            tcode,
            0xffffe0000000 + addr as u64,
            raw.len(),
            &mut raw,
            timeout_ms,
        )?;
        for (i, chunk) in raw.chunks_exact(4).enumerate() {
            frames[i] = u32::from_be_bytes(chunk.try_into().unwrap());
        }
        Ok(())
    }

    fn write(&mut self, addr: usize, frames: &[u32], timeout_ms: u32) -> Result<(), Error> {
        let mut raw = Vec::with_capacity(frames.len() * 4);
        for &frame in frames {
            raw.extend_from_slice(&frame.to_be_bytes());
        }
        let tcode = if frames.len() == 1 {
            FwTcode::WriteQuadletRequest
        } else {
            FwTcode::WriteBlockRequest
        };
        self.req.transaction_sync(
            &self.node,
            tcode,
            0xffffe0000000 + addr as u64,
            raw.len(),
            &mut raw,
            timeout_ms,
        )
    }
}
```

## 2. BeBoB Glue (`AvcOperators` trait)
For `Focusrite Saffire PRO 26 I/O`, use `hinawa::FwFcp`:

```rust
use hinawa::{FwFcp, FwFcpExt, FwFcpExtManual, FwNode};
use firewire_bebob_protocols::ta1394::general::AvcOperators;
use glib::Error;

pub struct BebobTransport {
    pub node: FwNode,
    pub fcp: FwFcp,
}

impl AvcOperators for BebobTransport {
    fn command(&mut self, operands: &[u8], timeout_ms: u32) -> Result<Vec<u8>, Error> {
        let mut resp = vec![0u8; 512];
        let len = self.fcp.avc_transaction(operands, &mut resp, timeout_ms)?;
        resp.truncate(len);
        Ok(resp)
    }
}
```
