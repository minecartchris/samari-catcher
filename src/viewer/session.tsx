import { Component, JSX, h } from "preact";
import { WebsocketCodes } from "../codes";
import { Capability, PacketCode, decodePacket, encodePacket } from "../network";
import type { Token } from "../token";
import { Computer } from "./computer";
import { BufferingEventQueue, PacketEvent } from "./event";
import { LostConnection, TokenDisplay, UnknownError } from "./screens";
import type { Settings } from "./settings";

export type SessionInfo = { id: number | null, label: string | null };

export type SessionProps = {
  token: Token,
  settings: Settings,
  focused: boolean,
  onInfo: (token: Token, info: SessionInfo) => void,
};

type SessionState = {
  websocket: WebSocket,
  events: BufferingEventQueue<PacketEvent>,

  hadConnected: boolean,
  currentVDom: (state: SessionState) => JSX.Element,
};

export class Session extends Component<SessionProps, SessionState> {
  public constructor(props: SessionProps, context: any) {
    super(props, context);
  }

  public override componentWillMount() {
    const { token } = this.props;
    const protocol = window.location.protocol === "http:" ? "ws:" : "wss:";
    const caps = [Capability.TerminalView, Capability.FileEdit].join(",");
    const socket = new WebSocket(`${protocol}//${window.location.host}/connect?id=${token}&capabilities=${caps}`);
    const events = new BufferingEventQueue<PacketEvent>();

    this.setState({
      websocket: socket,
      events,
      hadConnected: false,
      currentVDom: () => <TokenDisplay token={token} />,
    });

    socket.addEventListener("error", event => {
      if (socket.readyState <= WebSocket.OPEN) socket.close(400);
      console.error(event);
      this.setState({ currentVDom: () => <UnknownError error={`${event}`} /> });
    });

    socket.addEventListener("close", event => {
      console.error(event);
      this.setState({
        currentVDom: () => <UnknownError error="The socket was closed. Is your internet down?" />,
      });
    });

    socket.addEventListener("message", message => {
      const data = message.data;
      if (typeof data !== "string") return;

      const packet = decodePacket(data);
      if (!packet) {
        console.error("Invalid packet received");
        return;
      }

      switch (packet.packet) {
        case PacketCode.ConnectionUpdate: {
          const capabilities = new Set(packet.capabilities);
          if (capabilities.has(Capability.TerminalHost) || capabilities.has(Capability.FileHost)) {
            this.setState({
              currentVDom: this.computerVDom,
              hadConnected: true,
            });
          } else if (this.state.hadConnected) {
            this.setState({ currentVDom: () => <LostConnection token={token} /> });
            // Clear the info upstream so the tab label falls back to the token.
            this.props.onInfo(this.props.token, { id: null, label: null });
          } else {
            this.setState({ currentVDom: () => <TokenDisplay token={token} /> });
          }
          break;
        }

        case PacketCode.ConnectionAbuse:
          break;

        case PacketCode.ConnectionPing:
          socket.send(encodePacket({ packet: PacketCode.ConnectionPing }));
          break;

        case PacketCode.TerminalContents:
        case PacketCode.TerminalInfo:
        case PacketCode.FileAction:
        case PacketCode.FileConsume:
        case PacketCode.FileListing:
        case PacketCode.FileRequest:
          events.enqueue(new PacketEvent(packet));
          break;

        default:
          console.error("Unknown packet " + packet.packet);
          break;
      }
    });
  }

  public override componentWillUnmount() {
    const socket = this.state && this.state.websocket;
    if (socket) socket.close(WebsocketCodes.Normal);
  }

  public override shouldComponentUpdate(newProps: SessionProps, newState: SessionState) {
    return this.state.currentVDom !== newState.currentVDom ||
      this.props.settings !== newProps.settings ||
      this.props.focused !== newProps.focused;
  }

  public override render(_props: SessionProps, state: SessionState) {
    return state.currentVDom(state);
  }

  private computerVDom = ({ events, websocket }: SessionState) => {
    return <Computer events={events} connection={websocket} token={this.props.token}
      settings={this.props.settings} focused={this.props.focused}
      onInfo={info => this.props.onInfo(this.props.token, info)} />;
  }
}
