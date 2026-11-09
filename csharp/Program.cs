using System;
using System.Threading.Tasks;
using Rustwinui3loop;
using Windows.Foundation;

// Reg-free: the <activatableClass> entries ride on this exe's own manifest.
var connector = new NamedPipeConnector();
Console.WriteLine($"ctor worked, transport = {connector.Transport}");

var listener = new NamedPipeConnectionListener();
var tcs = new TaskCompletionSource<TransportStream>();

// A real C# event, which only exists because the Event row is now well formed.
listener.ConnectionReceived += (sender, args) => tcs.TrySetResult(args.Stream);

var pipe = $@"\\.\pipe\comio_cs_{Environment.ProcessId}";
await listener.BindAsync(pipe);
Console.WriteLine($"listening on {listener.Information}");

var client = await connector.ConnectAsync(pipe);
var server = await tcs.Task.WaitAsync(TimeSpan.FromSeconds(10));
Console.WriteLine("accepted via ConnectionReceived event");

var writer = new Windows.Storage.Streams.DataWriter(client);
writer.WriteString("hello from c#");
await writer.StoreAsync();
await writer.FlushAsync();

var reader = new Windows.Storage.Streams.DataReader(server);
var n = await reader.LoadAsync(64);
Console.WriteLine($"server read '{reader.ReadString(n)}'");

((IDisposable)listener).Dispose();
Console.WriteLine("CS-ROUND-TRIP-OK");
