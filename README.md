# Jamyx
![logo](assets/jamyx.png)
***

<p align="center">
<b><a href="#usage">Usage</a></b>
|
<b><a href="#installation">Installation</a></b>
|
<b><a href="#ipc">IPC Specs</a></b>
</p>

***
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

## IPC Specifications
Interprocess communication is done via a TCP connection to the Jamyx server (default port: `56065`). The messages are formatted in Json as follows:

### Format
**Command**
```json
{
    "target": "<TARGET>",      One of "myx" and "con" for targetting
                                the mixer and the patchbay respectively
    "cmd":    "<COMMAND>",     One of the later described commands
    "opts":   ["<OPTIONS>"]    The options for the chosen command
}
```

**Reply**
```json
{
    "ret": <RETURN CODE>        The return code of the command (0 = good)
    "msg": <MESSAGE>            Short description of return object or error
    "obj": <RETURN OBJECT TREE> Object tree caintaining information
                                 returned by the command
                                 (described in seperate command descs.)
}
```

## Commands (target: myx)
### con/dis/tog
Connect/Disconnect/Toggle two channels together

**Command**

|key|value|description|
|---|-----|----|
|target|`"myx"`|
|cmd|`"CMD"`|`con`, `dis`, or `tog` for connecting, disconnecting and toggling connection|
|opts|`["INPUT_NAME", "OUTPUT_NAME"]`|the names of the two channels

**Return object**

This command returns the [port object][1] of the output port

### get
Get port(s) specified

**Command**

|key|value|description|
|---|-----|----|
|target|`"myx"`|
|cmd|`"get"`|
|opts|`["monitor"]` **OR** `["channels"]` **OR** `["TYPE", "NAME"]`| get monitor channel **OR** get all in/output ports **OR** get specified channel where `TYPE` is `in` or `out` and `NAME` is the name of the channel|

**Return object**

This command returns [port object][1] of the specified port.

For the `get channels` command, this returns the following object:
```json
{
    "inputs": [OBJS],
    "outputs": [OBJS]
}
```
where the `OBJS` are [port objects][1]

### mon
Wait for a certain event and then return the

**Command**

|key|value|description|
|---|-----|----|
|target|`"myx"`|
|cmd|`"mon"`|monitor property on a certain port|
|opts|`["PROPERTY", "TYPE", "NAME"]`| where `PROPERTY` is any of `volume`, `connections`, or `balance`, **and** `TYPE` is `in` or `out` **and** `NAME` is the name of the channel|

**Return object**

This command returns [port object][1] of the specified port once the monitored property has changed.


### set
Set the value of a certain property

**Command**

|key|value|description|
|---|-----|----|
|target|`"myx"`|
|cmd|`"set"`|set the value of a property of a channel or set the monitor channel|
|opts|`["PROPERTY", "TYPE", "NAME", "VALUE"]`**OR**`["monitor", "TYPE", "NAME"]`|where `PROPERTY` is `volume` or `balance`, **and** `TYPE` is `in` or `out` **and** `NAME` is the name of the channel **and** `VALUE` is the new value|

**Return object**

This command returns [port object][1] of the specified port.

[1]: #port-object
