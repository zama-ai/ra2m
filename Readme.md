# Rust-Async Architecture Modeling (Ra2m)

*This repository contains the core of a Hardware Architecture Simulator based on
Rust Async.*

*The project short name must be pronounced "Erratum". It's a small joke around
the expected benefits of an architecture simulator. Indeed, objective of
architecture simulation is to provide good insight on the achieved performances
that will prevent "Erratum" in the design stages and thus a time saving.*


## Project structure
Ra2m is split in 3 parts:
 * ra2m_sim: contains ra2m simulation kernel
 * ra2m_cpn: library of components modeled with ra2m_sim
 * tools: contains a parsers to convert Ra2m trace in polars dataframe for
     analyses

### Ra2m_sim
This crate contains all the required object to run a simulation of hardware
component. 
It hook hardware simulation time inside the tokio event scheduler. A module and port abstraction 
are defined and implemented alongside a variety of utilities types/tools.

A `Module` could have multiple `Port` to communicate with other `Module`s.
The internal behavior of a `Module` could be described with multiple concurrent
process.
Every process could be halted at any position:
 * for a given amount of simulated time `delay::Delay::wait_for(...)`
 * Until the given simulated time `delay::Delay::wait_until(...)`
 * Until a given event occurred `event::Event::wait(...)`

Various `Port` implementation could be used from half_duplex to full_duplex with
session management (i.e. multiple stream could be interleaved through a port and
easily dispatch back to issuers process).
Full_duplex implementation is build upon two half_duplex port.
The following high-level API is exposed to the user:
* half_duplex:
    * `send_pkt[_burst](...)` -> Send a packet or a burst of packets.
        Contains checker for packet status (Packet must be correctly shaped for
        a communication start
    * `fwd_pkt[_burst](...)` -> Send a packet or a burst of packets. No
        checkers, used for middle point communication handling
    * `wait_pkt[_burst]()` -> Wait a packet or a burst of packets. Use by
        middle point (i.e. no check, time check, and trace dump)
    * `wait_pkt[_burst]_ep()` -> Wait a packet or a burst of packets. Use by
        communication endpoint. A set of check are done and associated Packet
        are properly teardown.
    * ... Cf. `cargo doc` for more details on the API 
* full_duplex: (Two flavor depending of Port kind i.e. w/wo stream dispatch)
    * `b_req_resp[_burst](...)` -> Issue a request and block until the response
        is received.
        Use to simply generate blocking end to end communication.
        More fine grain communication could be achieved by using directly
        half_duplex API over internal rx/tx endpoint
    * `wait_req_forge_resp(..)` -> Wait a request and forge the response based
        on a given closure.
    * ... Cf. `cargo doc` for more details on the API 

Ra2m_sim also provide a highly tweak-able logging/tracing structure. Log
messages belong to a Category and a Verbosity level.
At execution time user could choose to set the Verbosity level per Category and
component hierarchy:
```
--log-trace "regionA::SubregionB::CpnC=Info:{Port,Debug}
```

Will log messages with Verbosity >= Info  for all Category and >= Debug for Port
Category in the module CpnC deep in the hierarchy.
This syntax also support regex for multiple component path/name matching.

Trace are also generated on Category/Module basis. Every component generate
trace in it's own dedicated file.


### Ra2m_cpn
Contains a set of common component that could be assembled to describe a more
complex architecture.
Not very well supplied for the moment, but should grow in the near future.

## Examples
The examples folder contains a set a code snippets showing the way to use `ra2m`
for architecture modeling:
 * `addr_map`: Simple example that associate traffic generator, Xbar and
     Memories.
 * `gen_soc`: Generic SoC

The following code snippets depict how to start the examples:
``` bash
# Display the addr_map example options
cargo run --release --example addr_map -- --help

# Run the addr_map example for 10_us in loosely-timed mode (other options is
# kept as default)
cargo run --release --example addr_map -- --duration 10_us --timing-mode LT
```
