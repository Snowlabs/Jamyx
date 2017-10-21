# Jamyx

A Jackaudio mixer/patchbay suite written in rust

Jamyx is meant to be an easy to use alternative to qjackctl.



## Why should I replace qjackctl?

### Jamyx is a daemon

Because Jamyx is a server and requires no windows manager or graphics to run. This means that even if you restart your X server, the Jamyx server daemon will keep running independently. Why should your audio setup depend on your window manager? This is also why Jamyx is called `Jamyx` and not `qJamyx` of `Jamyx-gtk`. If you want a graphical front-end to Jamyx, you can download a separate client or even make your own!

### Jamyx has more features

Jamyx offers a patchbay utility as well as a full featured audio mixer! After some quick configuration, you can have complex audio routes and volume control over your ports.

Some key features include:

**Mixer**:

- Individual volume and balance control over ports
- Support for mono and stereo ports
- Supper for monitor channel
- Connection of volume-controlled ports to volume-controlled outputs in a grid-like system

**Patchbay**:

- Automatic event-based (dis)connections of ports when they appear
- Automatic retrial of (dis)connections when they fail
- Multithreaded and event-based, so never skips a beat!

**General**:

- On-the-fly loading and saving of current configuration state with non need to restart the daemon.
- Jack server reconnection loop. This means the Jamyx server is independant of the Jackaudio server and will wait for it to restart in case it stops or crashes. Crashing or stopping the Jack server will not crash the Jamyx server.


- Multithreaded IPC


- Easy IPC for server-client communication:

  - Monitor any port property and event (e.g.: wait for change in volume of a certain port)

  - Getters & Setters for all properties

  - Create, delete and rename volume-controlled ports

  - Json is used for easy (de)serializing of commands and replies

    ​