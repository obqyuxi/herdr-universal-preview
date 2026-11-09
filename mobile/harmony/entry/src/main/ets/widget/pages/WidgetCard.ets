// 码上 on the home screen: the agent that waits for you and its choices.
// Values come from ets/core/Cards.ets (app) or bridge/src/card.rs (Push Kit).
const storage = new LocalStorage();

@Entry(storage)
@Component
struct WidgetCard {
  @LocalStorageProp('formId') formId: string = '';
  @LocalStorageProp('n') n: string = '';
  @LocalStorageProp('head') head: string = '';
  @LocalStorageProp('q') q: string = '';
  @LocalStorageProp('o1') o1: string = '';
  @LocalStorageProp('o2') o2: string = '';
  @LocalStorageProp('o3') o3: string = '';
  @LocalStorageProp('i1') i1: string = '';
  @LocalStorageProp('i2') i2: string = '';
  @LocalStorageProp('i3') i3: string = '';
  @LocalStorageProp('addr') addr: string = '';
  @LocalStorageProp('note') note: string = '';

  private open(): void {
    postCardAction(this, {
      action: 'router',
      abilityName: 'EntryAbility',
      params: { terminal: this.addr }
    });
  }

  private answer(opt: string, label: string): void {
    postCardAction(this, {
      action: 'call',
      abilityName: 'EntryAbility',
      params: { method: 'answer', formId: this.formId, addr: this.addr, opt: opt, label: label }
    });
  }

  @Builder
  choice(label: string, opt: string, first: boolean) {
    Button(label)
      .fontSize(13)
      .fontColor(first ? '#101114' : '#E6E6E6')
      .backgroundColor(first ? '#FFB547' : '#2A2D33')
      .height(32)
      .layoutWeight(1)
      .padding({ left: 4, right: 4 })
      .onClick(() => this.answer(opt, label))
  }

  build() {
    Column({ space: 6 }) {
      Row({ space: 6 }) {
        Image($r('app.media.app_icon')).width(18).height(18).borderRadius(4)
        Text(this.n !== '' && this.n !== '0' ? `${this.n} 个在等你` : '码上')
          .fontSize(12).fontColor(this.n !== '' && this.n !== '0' ? '#FFB547' : '#9AA0A6')
        Blank()
        if (this.head) {
          Text(this.head).fontSize(12).fontColor('#9AA0A6').maxLines(1)
            .textOverflow({ overflow: TextOverflow.Ellipsis }).constraintSize({ maxWidth: '60%' })
        }
      }.width('100%')

      if (this.addr) {
        Text(this.q).fontSize(14).fontColor('#E6E6E6').fontWeight(FontWeight.Medium)
          .maxLines(this.o1 ? 2 : 3).textOverflow({ overflow: TextOverflow.Ellipsis }).width('100%')
        Blank()
        if (this.o1) {
          Row({ space: 6 }) {
            this.choice(this.o1, this.i1, true)
            if (this.o2) {
              this.choice(this.o2, this.i2, false)
            }
            if (this.o3) {
              this.choice(this.o3, this.i3, false)
            }
          }.width('100%')
        }
        if (this.note) {
          Text(this.note).fontSize(11).fontColor('#8A8F98').maxLines(1)
            .textOverflow({ overflow: TextOverflow.Ellipsis }).width('100%')
        }
      } else {
        Blank()
        Text(this.note ? this.note : '没有在等你的任务').fontSize(14).fontColor('#C9CDD2')
        Blank()
      }
    }
    .width('100%').height('100%').padding(12)
    .backgroundColor('#15171B')
    .onClick(() => this.open())
  }
}
