import 'dart:async';
import 'dart:convert';
import 'dart:math' as math;

import 'package:flutter/material.dart';
import 'package:flutter/services.dart';

const up = Color(0xffeb3e3e);
const down = Color(0xff22b05f);
const ink = Color(0xff1f2430);

Future<void> main() async {
  WidgetsFlutterBinding.ensureInitialized();
  final data =
      jsonDecode(await rootBundle.loadString('assets/market.json'))
          as Map<String, dynamic>;
  runApp(Market(data));
}

class Market extends StatefulWidget {
  const Market(this.data, {super.key});

  final Map<String, dynamic> data;

  @override
  State<Market> createState() => _MarketState();
}

class _MarketState extends State<Market> with WidgetsBindingObserver {
  final frame = ValueNotifier(0);
  final running = ValueNotifier(true);
  late final List<dynamic> frames = widget.data['frames'] as List<dynamic>;
  late final Timer timer;
  bool visible = true;

  @override
  void initState() {
    super.initState();
    WidgetsBinding.instance.addObserver(this);
    final period = widget.data['periodMillis'] as int;
    timer = Timer.periodic(Duration(milliseconds: period), (_) {
      if (visible && running.value)
        frame.value = (frame.value + 1) % frames.length;
    });
  }

  @override
  void didChangeAppLifecycleState(AppLifecycleState state) {
    visible =
        state == AppLifecycleState.resumed ||
        state == AppLifecycleState.inactive;
  }

  @override
  void dispose() {
    timer.cancel();
    WidgetsBinding.instance.removeObserver(this);
    frame.dispose();
    running.dispose();
    super.dispose();
  }

  Widget marketButton() => ValueListenableBuilder(
    valueListenable: running,
    builder: (context, live, _) => IconButton(
      tooltip: live ? '暂停行情' : '继续行情',
      onPressed: () => running.value = !live,
      icon: Icon(live ? Icons.pause : Icons.play_arrow),
    ),
  );

  @override
  Widget build(BuildContext context) {
    final instruments = widget.data['instruments'] as List<dynamic>;
    final detailIndex = widget.data['detailIndex'] as int;
    return MaterialApp(
      title: '行情参考',
      theme: ThemeData(
        colorSchemeSeed: ink,
        scaffoldBackgroundColor: Colors.white,
      ),
      home: Scaffold(
        appBar: AppBar(title: const Text('市场行情'), actions: [marketButton()]),
        body: ListView.builder(
          itemCount: instruments.length,
          itemExtent: 62,
          itemBuilder: (context, index) {
            final instrument = instruments[index] as Map<String, dynamic>;
            return InkWell(
              onTap: index == detailIndex
                  ? () => Navigator.of(context).push(
                      MaterialPageRoute<void>(
                        builder: (_) => QuoteDetail(
                          instrument['name'] as String,
                          widget.data['daily'] as List<dynamic>,
                          frames,
                          frame,
                          marketButton(),
                        ),
                      ),
                    )
                  : null,
              child: ValueListenableBuilder(
                valueListenable: frame,
                builder: (_, current, _) {
                  final row = frames[current]['rows'][index] as List<dynamic>;
                  final change = row[1] as num;
                  final color = change >= 0 ? up : down;
                  return Padding(
                    padding: const EdgeInsets.symmetric(horizontal: 16),
                    child: Row(
                      children: [
                        Expanded(
                          child: Column(
                            mainAxisAlignment: MainAxisAlignment.center,
                            crossAxisAlignment: CrossAxisAlignment.start,
                            children: [
                              Text(instrument['name'] as String),
                              Text(
                                instrument['symbol'] as String,
                                style: Theme.of(context).textTheme.bodySmall,
                              ),
                            ],
                          ),
                        ),
                        CustomPaint(
                          size: const Size(96, 36),
                          painter: MarketChart(row[2] as List, color: color),
                        ),
                        SizedBox(
                          width: 84,
                          child: Column(
                            mainAxisAlignment: MainAxisAlignment.center,
                            crossAxisAlignment: CrossAxisAlignment.end,
                            children: [
                              Text((row[0] as num).toStringAsFixed(2)),
                              Text(
                                '${change >= 0 ? '+' : ''}${change.toStringAsFixed(2)}%',
                                style: TextStyle(color: color),
                              ),
                            ],
                          ),
                        ),
                      ],
                    ),
                  );
                },
              ),
            );
          },
        ),
      ),
    );
  }
}

class QuoteDetail extends StatefulWidget {
  const QuoteDetail(
    this.name,
    this.daily,
    this.frames,
    this.frame,
    this.marketButton, {
    super.key,
  });

  final String name;
  final List<dynamic> daily;
  final List<dynamic> frames;
  final ValueNotifier<int> frame;
  final Widget marketButton;

  @override
  State<QuoteDetail> createState() => _QuoteDetailState();
}

class _QuoteDetailState extends State<QuoteDetail> {
  bool daily = false;
  double? crosshair;

  @override
  Widget build(BuildContext context) => Scaffold(
    appBar: AppBar(title: Text(widget.name), actions: [widget.marketButton]),
    body: ValueListenableBuilder(
      valueListenable: widget.frame,
      builder: (context, index, _) {
        final detail = widget.frames[index]['detail'] as Map<String, dynamic>;
        final quote = detail['quote'] as Map<String, dynamic>;
        String price(String key) => (quote[key] as num).toStringAsFixed(2);
        final chart = daily ? widget.daily : detail['minute'] as List<dynamic>;
        final theme = Theme.of(context).textTheme;
        return ListView(
          padding: const EdgeInsets.fromLTRB(16, 12, 16, 32),
          children: [
            Text(price('last'), style: theme.headlineLarge),
            const SizedBox(height: 12),
            Text(
              '最高 ${price('high')}   最低 ${price('low')}   开盘 ${price('open')}',
            ),
            const SizedBox(height: 16),
            SegmentedButton<bool>(
              segments: const [
                ButtonSegment(value: false, label: Text('分时')),
                ButtonSegment(value: true, label: Text('日 K')),
              ],
              selected: {daily},
              onSelectionChanged: (value) => setState(() {
                daily = value.single;
                crosshair = null;
              }),
            ),
            const SizedBox(height: 16),
            GestureDetector(
              behavior: HitTestBehavior.opaque,
              onHorizontalDragUpdate: (event) =>
                  setState(() => crosshair = event.localPosition.dx),
              onHorizontalDragEnd: (_) => setState(() => crosshair = null),
              onHorizontalDragCancel: () => setState(() => crosshair = null),
              child: SizedBox(
                height: 204,
                child: CustomPaint(
                  painter: MarketChart(
                    chart,
                    candles: daily,
                    crosshair: crosshair,
                  ),
                ),
              ),
            ),
            const Divider(height: 24),
            Text('盘口', style: theme.titleMedium),
            for (final side in (detail['book'] as Map<String, dynamic>).entries)
              if (side.value is List)
                for (final level in side.value as List)
                  ListTile(
                    dense: true,
                    title: Text('${level['price']}'),
                    trailing: Text('${level['size']}'),
                  ),
            const Divider(height: 24),
            Text('逐笔成交', style: theme.titleMedium),
            for (final trade in detail['trades'] as List)
              ListTile(
                dense: true,
                title: Text('${trade['price']}'),
                trailing: Text('${trade['volume']}'),
              ),
          ],
        );
      },
    ),
  );
}

class MarketChart extends CustomPainter {
  const MarketChart(
    this.series, {
    this.candles = false,
    this.crosshair,
    this.color = const Color(0xff375ba3),
  });

  final List<dynamic> series;
  final bool candles;
  final double? crosshair;
  final Color color;

  @override
  void paint(Canvas canvas, Size size) {
    if (series.isEmpty || size.isEmpty) return;
    final prices = candles
        ? series.cast<List>().expand((bar) => bar.cast<num>())
        : series.cast<num>();
    var low = double.infinity;
    var high = double.negativeInfinity;
    for (final price in prices) {
      low = math.min(low, price.toDouble());
      high = math.max(high, price.toDouble());
    }
    final span = math.max(0.01, high - low);
    final slot = size.width / series.length;
    double y(num value) =>
        size.height - 4 - (size.height - 8) * (value - low) / span;
    double x(int index) => (index + 0.5) * slot;
    final paint = Paint()..color = color;
    if (candles) {
      for (var index = 0; index < series.length; index++) {
        final bar = (series[index] as List).cast<num>();
        final open = y(bar[0]);
        final close = y(bar[3]);
        final center = x(index);
        paint.color = bar[3] >= bar[0] ? up : down;
        canvas.drawLine(
          Offset(center, y(bar[1])),
          Offset(center, y(bar[2])),
          paint,
        );
        canvas.drawRect(
          Rect.fromLTWH(
            center - slot * 0.3,
            math.min(open, close),
            slot * 0.6,
            math.max(1.0, (open - close).abs()),
          ),
          paint,
        );
      }
    } else {
      final path = Path()..moveTo(x(0), y(series[0] as num));
      for (var index = 1; index < series.length; index++) {
        path.lineTo(x(index), y(series[index] as num));
      }
      canvas.drawPath(
        path,
        paint
          ..style = PaintingStyle.stroke
          ..strokeWidth = 1.2,
      );
    }
    final position = crosshair;
    if (position != null) {
      canvas.drawLine(
        Offset(position, 0),
        Offset(position, size.height),
        Paint()..color = ink,
      );
    }
  }

  @override
  bool shouldRepaint(MarketChart old) =>
      series != old.series ||
      candles != old.candles ||
      crosshair != old.crosshair ||
      color != old.color;
}
