# ftx-async
Unofficial Rust implementation of an asynchronous Websocket and REST client for the FTX crypto exchange 

![ci](https://github.com/IanMichaelAsh/ftx-async/actions/workflows/ci.yml/badge.svg)

>02/11/2022 - *** WARNING *** Code is currently being ported into this repository. You are welcome to use it as a reference, however you should have no expectation that it builds or runs correctly. The basic port is now complete, see the tests directly for very basic examples of utilising the websocket and REST modules.


<h1>Running Integration Tests</h1>
Integration tests use environment variables to look up credentials to use to authenticate with the FTX exchange. A read-only key should be created on FTX and its details should be set into 'FTX_API_KEY' and 'FTX_SECRET' environment variables.

