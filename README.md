# perceive
Semantic search for your life

## Building on macOS

You'll need to have libtorch installed. You can use `pip3` or `homebrew` to install `pytorch`, and then
set the LIBTORCH environment variable to the path where libtorch is installed.

For example, when installed via homebrew you would add `export LIBTORCH=/opt/homebrew/opt/pytorch` to your `~/.zshrc` or
`~/.zprofile`.

At some point I'll try to add some better way of installing a local copy too, to make it easier to match the version of
libtorch with the version expected by the bindings.
