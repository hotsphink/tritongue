import plugins

import random
import re


# Plagiarized from http://humorlistblog.blogspot.com/2012/01/top-50-puns.html
# and http://www.angelfire.com/falcon/jenza/humour/list/puns.html

JOKES = """
2. I'm reading a book about anti-gravity. It's impossible to put down.

9. A hole has been found in the nudist camp wall. The police are looking into it.

31. Sleeping comes so naturally to me, I could do it with my eyes closed.
""".split("\n\n")


PUNS = """\
1. the lua programming community once considered adding a new data structure<...3sec...>in the end, they tabled that idea

1. My battery had an alkaline problem, so it went to AA meetings

2. Herb gardeners who work extra get thyme and a half

3. Atoms are untrustworthy.<...2sec...>They make up everything.

4. Why was Cinderella kicked off the soccer team?<...2sec...>She always ran away from the ball.

5. I sued the airline after it misplaced my luggage.<...2sec...>Unfortunately, I lost my case.

6. Sea monsters eat fish and ships.

7. Why was the tomato red?<...2sec...>It saw the salad dressing.

8. I was recently diagnosed with color-blindness. It came out of the green.<...4sec...>Sorry, I know that's not a pun.

9. The future, the present, and the past walk into a shady bar.<...1sec...>Things get tense.

10. SpaceX plans to open a new restaurant on the moon<...4sec...>they say the food will be great, but I'm worried about the lack of atmosphere

11. I'm not too worried that I hurt myself shredding cheese for my pasta.<...2sec...>I have grater problems.

12. What do you call your sweetheart when she loses her eyes?<...2sec...>No idea.

13. Why do ghosts ride elevators?<...2sec...>To lift their spirits.

8. I used to have a fear of hurdles, but I got over it.

19. A new type of broom came out, it is sweeping the nation.

1. I wondered why the baseball was getting bigger. Then it hit me.

3. Did you hear about the guy whose whole left side was cut off?<...2sec...>He's all right now.

5. I forgot where I threw my boomerang, but eventually it came back to me.

6. There was a sign on the lawn at a drug re-hab center that said 'Keep off the Grass'.

7. I was going to look for my missing watch, but I could never find the time.

10. Police were called to a daycare where a three-year-old was resisting a rest.

11. To write with a broken pencil is pointless.

12. He drove his expensive car into a tree and found out how the Mercedes bends.

13. Atheism is a non-prophet organization.

14. I did a theatrical performance about puns. Really it was just a play on words.

15. I used to be addicted to soap, but I'm clean now.

16. Show me a piano falling down a mineshaft and I'll show you A-flat minor.

17. A bicycle can't stand on its own because it is two tired.

18. Need an ark to save two of every animal? I noah guy.

20. A small boy swallowed some coins and was taken to a hospital. When his grandmother telephoned to ask how he was, a nurse said 'No change yet'.

21. The new weed whacker is cutting-hedge technology.

22. Some people's noses and feet are built backwards: their feet smell and their noses run.

23. When William joined the army he disliked the phrase 'fire at will'.

24. Did you hear about the guy who got hit in the head with a can of soda? He was lucky it was a soft drink.

25. There was once a cross-eyed teacher who couldn't control his pupils.

26. The butcher backed up into the meat grinder and got a little behind in his work.

27. I wanted to lose weight so I went to the paint store. I heard I could get thinner there.

28. Lightning sometimes shocks people because it just doesn't know how to conduct itself.

29. A prisoner's favorite punctuation mark is the period. It marks the end of his sentence.

30. A rule of grammar: double negatives are a no-no.

32. Time flies like an arrow. Fruit flies like a banana.

33. Atheists don't solve exponential equations because they don't believe in higher powers.

34. It's raining cats and dogs. Well, as long as it doesn't reindeer.

35. I relish the fact that you've mustard the strength to ketchup to me.

36. My new theory on inertia doesn't seem to be gaining momentum.

37. I was going to buy a book on phobias, but I was afraid it wouldn't help me.

38. The man who survived mustard gas and pepper spray is now a seasoned veteran.

39. What did the grape say when it got stepped on? Nothing - but it let out a little whine.

40. When a clock is hungry it goes back four seconds.

41. If you don't pay your exorcist you get repossessed.

42. She got fired from the hot dog stand for putting her hair in a bun.

43. The dead batteries were given out free of charge.

44. John Deere's manure spreader is the only equipment the company won't stand behind.

45. Pencils could be made with erasers at both ends, but what would be the point?

46. I was arrested after my therapist suggested I take something for my kleptomania.

47. A hungry traveller stops at a monastery and is taken to the kitchens. A brother is frying chips. 'Are you the friar?' he asks. 'No. I'm the chip monk,' he replies.

48. Yesterday I accidentally swallowed some food coloring. The doctor says I'm OK, but I feel like I've dyed a little inside.

49. What's the definition of a will? (It's a dead giveaway).

50. Two peanuts were walking in a tough neighborhood and one of them was a-salted.

5. Those who get too big for their britches will be exposed in the end.

7. A chicken crossing the road is poultry in motion.

10. The man who fell into an upholstery machine is fully recovered.

11. Every calendar's days are numbered.

12. Bakers trade bread recipes on a knead to know basis.

13. When the electricity went off during a storm at a school the students were de-lighted.

14. I used to be a tap dancer until I fell in the sink.

16. She was only a whisky maker but he loved her still.

17. She had a boyfriend with a wooden leg, but broke it off.

19. It wasn't school John disliked it was just the principal of it.

22. A grenade thrown into a kitchen in France would result in Linoleum Blownapart.

23. A boiled egg in the morning is hard to beat.

25. Old power plant workers never die they just de-generate.

26. There was a ghost at the hotel, so they called for an inn spectre.

27. With her marriage she got a new name and a dress.

28. The short fortune-teller who escaped from prison was a small medium at large

29. Some Spanish government employees are Seville servants.

33. When cannibals ate a missionary they got a taste of religion.

34. When an actress saw her first strands of gray hair she thought she'd dye.

35. He often broke into song because he couldn't find the key.

36. Marathon runners with bad footwear suffer the agony of defeat.

37. Driving on so many turnpikes was taking its toll.

38. To some - marriage is a word ... to others - a sentence.

39. Old lawyers never die they just lose their appeal.

40. In democracy it's your vote that counts. In feudalism it's your Count that votes.

42. It was an emotional wedding. Even the cake was in tiers.

43. Old skiers never die -- they just go downhill.

44. A cardboard belt would be a waist of paper.

45. Local Area Network in Australia: the LAN down under.

46. When the TV repairman got married the reception was excellent.

47. An office with many people and few electrical outlets could be in for a power struggle.

48. How do you make antifreeze?<...2sec...>Steal her blanket.

50. A pediatrician is a doctor of little patients.

51. Nylons give women a run for their money.

53. Ancient orators tended to Babylon.

56. Two silk worms had a race. They ended up in a tie.

57. He had a photographic memory that was never developed.

58. Old burglars never die they just steal away.

59. A lawyer for a church did some cross-examining.

61. Some people don't like food going to waist..

63. You feel stuck with your debt if you can't budge it.

64. Women who no longer get asked out as often as their younger friends could feel out-dated.

66. A pet store had a bird contest with no perches necessary.

67. A backwards poet writes inverse.

68. If a lawyer can be disbarred can a musician be denoted or a model deposed?

69. Once you've seen one shopping center you've seen the mall.

71. A plateau is a high form of flattery.

72. When chemists die, we barium.

74. When the wheel was invented, it caused a revolution.

75. Two robbers with clubs went golfing, but they didn't play the fairway.

76. Seven days without a pun makes one weak.

77. A circus lion won't eat clowns because they taste funny.

78. A toothless termite walked into a tavern and said, "Is the bar tender here?"

79. Did you hear about the fire at the circus? The heat was intense.

81. Santa's helpers are subordinate clauses.

82. A lot of money is tainted. It taint yours and it taint mine.

83. When they bought a water bed, the couple started to drift apart.

84. What you seize is what you get.

86. Some people are built backwards: their feet smell and their noses run.

87. Two banks with different rates have a conflict of interest.

89. What do you call cheese that is not yours?<...2sec...>Nacho Cheese.

90. When a new hive is done, bees have a house swarming party.

92. Never lie to an X-ray technician. They can see right through you.

93. Old programmers never die, they just can't C as well.

95. Long fairy tales have a tendency to dragon.

98. A ditch digger was entrenched in his career.

99. A girl and her boyfriend went to a party as a barcode. They were an item.

100. A criminal's best asset is his lie ability.

13. Have you ever tried eating a clock? It's very time consuming.

23. I'm glad I know sign language, it's pretty handy.

24. The experienced carpenter really nailed it, but the new guy screwed everything up.

31. What is the difference between a nicely dressed man on a tricycle and a poorly dressed man on a bicycle?<...2sec...>A tire.

32. I saw a documentary on beavers last night, it was the best dam movie I've ever seen.

40. The other day a clown held the door for me. I thought it was a nice jester.

55. The cannibal showed up late to the luncheon, so they gave him the cold shoulder.

57. When I entered the building I thought using the elevator was really uplifting, but then it let me down.

64. Two hats were hanging on a hat rack in the hallway. One hat says to the other, 'You stay here, I'll go on a head.'

69. I don't mind kids playing hopscotch in most places, but my driveway is where I draw the line.

75. After 8 years of trying, Einstein developed a theory about space. It was about time, too.

76. Broken puppets for sale. No strings attached.

83. My tailor is happy to make a pair of pants for me, or at least sew its seams.

92. I think every morning that I'm going to make pancakes, but I keep waffling.

100. I try wearing a pair of tight jeans, but I can never pull it off.

101. I should have been sad when my flashlight batteries died, but I was delighted.

13. Did you hear about the fight at a local laundromat? A washing machine beat the crap out of a diaper.

I can't believe I got fired from the calendar factory. All I did was take a day off.

I tried working in an orange juice factory, but I got canned because I couldn't concentrate.

The shepherd sent his dog to collect about 3 dozen sheep. After the dog rounded them up, the shepherd sold all 40.

Don't yell through a screen door, you might strain your voice.

I wasn't originally going to get a brain transplant, but then I changed my mind.

Why don't some couples go to the gym?<...2sec...>Because some relationships don't work out.

Have you ever tried to eat a clock?<...2sec...>It's very time consuming.

My first job was as a banker, and when I started out I really enjoyed it. But after I lost interest, I pretty much had to leave.

I once got into so much debt that I couldn't even afford my electricity bills.<...3sec...>Those were the darkest days of my life.

Freezing rain might not be made of ice, but it sure hurts like hail.

My job at the concrete plant seems to get harder and harder.

Why did Joe quit his job at the doughnut factory?<...2sec...>He probably got tired of the hole business.

Light travels faster than sound.<...4sec...>That's why some people appear bright until you hear them speak.
""".split("\n\n")

class PunPlugin(plugins.Plugin):
    '''Lighthearted alternative to 'ping' command'''

    @plugins.command('pun', direct=True, pattern='^pun$')
    def tell_pun(self, room, event_id, caps):
        '''tell a randomly chosen pun'''
        filename = getattr(caps, 'substr', '')
        if filename == '':
            pun = random.sample(PUNS, 1)[0]
        else:
            matches = [p for p in PUNS if filename in p]
            pun = random.sample(matches, 1)[0]
        pun = re.sub(r'^\d+\. ', '', pun)
        return pun
