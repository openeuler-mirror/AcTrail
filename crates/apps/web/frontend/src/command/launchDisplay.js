// Command and exec actions describe a process image launch, not its lifetime.
export class CommandLaunchDisplay {
  constructor(action) {
    this.action = action;
  }

  get status() {
    switch (this.action?.status) {
      case 'success': return 'Started';
      case 'error': return 'Start failed';
      default: return 'Start result unobserved';
    }
  }
}
